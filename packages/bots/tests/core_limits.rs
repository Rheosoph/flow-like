use std::sync::{Arc, Mutex};

use flow_like_bots::limits::{
    BOT_RUNS_PER_MINUTE, CHAT_RUNS_PER_MINUTE, CHAT_WAITING, ChatQueues, HOUR_MS, HourBudget,
    IMAGE_BYTES, IMAGE_BYTES_PER_HOUR, ImageBudget, MINUTE_MS, NoticeKind, Notices, Offer, Rates,
    Restarts,
};

#[test]
fn ten_runs_a_minute_per_chat() {
    let mut rates = Rates::new();
    for second in 0..CHAT_RUNS_PER_MINUTE as u64 {
        assert!(rates.admit("chat", second * 1_000), "start {second}");
    }
    assert!(!rates.admit("chat", 30_000));
    assert!(rates.admit("other", 30_000));
    assert!(!rates.admit("chat", MINUTE_MS - 1));
    assert!(rates.admit("chat", MINUTE_MS));
    assert!(!rates.admit("chat", MINUTE_MS + 500));
    assert!(rates.admit("chat", MINUTE_MS + 1_000));
}

#[test]
fn sixty_runs_a_minute_per_bot() {
    let mut rates = Rates::new();
    for chat in 0..BOT_RUNS_PER_MINUTE {
        assert!(rates.admit(&chat.to_string(), 1_000), "chat {chat}");
    }
    assert!(!rates.admit("fresh", 2_000));
    assert!(!rates.admits("fresh", MINUTE_MS));
    assert!(rates.admits("fresh", MINUTE_MS + 1_000));
}

#[test]
fn a_refused_start_is_not_counted() {
    let mut rates = Rates::new();
    for second in 0..CHAT_RUNS_PER_MINUTE as u64 {
        rates.admit("chat", second);
    }
    for _ in 0..100 {
        assert!(!rates.admit("chat", 10_000));
    }
    assert!(rates.admit("chat", MINUTE_MS + 10));
}

#[test]
fn one_running_and_four_waiting_per_chat() {
    let mut queues = ChatQueues::new();
    assert_eq!(queues.offer("a", 0), Offer::Start(0));
    for item in 1..=CHAT_WAITING {
        assert_eq!(queues.offer("a", item), Offer::Queued);
    }
    assert_eq!(queues.offer("a", 5), Offer::Full(5));
    assert_eq!(queues.offer("b", 6), Offer::Start(6));
    assert!(queues.busy("a") && queues.busy("b"));
    assert_eq!(queues.waiting("a"), CHAT_WAITING);
    assert_eq!(queues.next("a"), Some(1));
    assert_eq!(queues.offer("a", 7), Offer::Queued);
    assert_eq!(queues.next("b"), None);
    assert!(!queues.busy("b"));
    let mut drained: Vec<usize> = queues
        .drain_waiting()
        .into_iter()
        .map(|(chat, item)| {
            assert_eq!(chat, "a");
            item
        })
        .collect();
    drained.sort_unstable();
    assert_eq!(drained, [2, 3, 4, 7]);
    assert!(queues.busy("a"));
    assert_eq!(queues.next("a"), None);
    assert!(!queues.busy("a"));
}

#[test]
fn one_notice_per_chat_kind_and_minute() {
    let mut notices = Notices::default();
    assert!(notices.allow("a", NoticeKind::Busy, 0));
    assert!(!notices.allow("a", NoticeKind::Busy, 59_999));
    assert!(notices.allow("a", NoticeKind::Image, 1));
    assert!(notices.allow("b", NoticeKind::Busy, 2));
    assert!(notices.allow("a", NoticeKind::Busy, MINUTE_MS));
    for kind in [NoticeKind::Busy, NoticeKind::Image, NoticeKind::Restart] {
        assert!(!kind.text().is_empty() && kind.text().len() < 80);
    }
}

#[test]
fn two_images_a_run_of_two_mebibytes() {
    let mut images = ImageBudget::fresh();
    assert!(images.allows(IMAGE_BYTES));
    assert!(!images.take(IMAGE_BYTES + 1));
    assert!(images.take(IMAGE_BYTES));
    assert!(images.take(10));
    assert!(!images.allows(1));
    assert!(!images.take(1));
    images.leave_out();
    assert_eq!((images.taken(), images.left_out()), (2, 3));
}

#[test]
fn image_bytes_per_bot_and_hour() {
    let hour = Arc::new(Mutex::new(HourBudget::new()));
    let fit = IMAGE_BYTES_PER_HOUR / IMAGE_BYTES;
    let mut taken = 0;
    for run in 0..fit {
        let mut images = ImageBudget::new(hour.clone(), run * 1_000);
        assert!(images.take(IMAGE_BYTES), "run {run}");
        taken += 1;
    }
    assert_eq!(taken, 128);
    let mut late = ImageBudget::new(hour.clone(), HOUR_MS - 1);
    assert!(!late.take(1));
    assert_eq!(late.left_out(), 1);
    let mut next_hour = ImageBudget::new(hour, HOUR_MS);
    assert!(next_hour.take(IMAGE_BYTES));
}

/// A connection that ends at once is started again after growing pauses, and never more than
/// twenty times within any hour, even when its pauses start small again.
#[test]
fn restarts_are_bounded() {
    for settles in [false, true] {
        let mut restarts = Restarts::new();
        let mut now = 0;
        let mut starts = Vec::new();
        while now < 3 * HOUR_MS {
            restarts.started(now);
            starts.push(now);
            if settles {
                restarts.settled();
            }
            now += restarts.pause_ms(now);
        }
        for (index, start) in starts.iter().enumerate() {
            let within = starts[index..]
                .iter()
                .take_while(|later| **later < start + HOUR_MS)
                .count();
            assert!(
                within <= 20,
                "settles {settles}: {within} starts from {start}"
            );
        }
        if !settles {
            assert_eq!(
                starts[..8],
                [0, 5_000, 15_000, 35_000, 75_000, 155_000, 315_000, 615_000]
            );
        }
    }
}
