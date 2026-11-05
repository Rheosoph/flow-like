use super::{Change, fixture};
use crate::{LOCAL_OWNER_ID, codes};

#[test]
fn a_grant_points_one_way_and_granting_again_keeps_the_first() {
    let fixture = fixture();
    let scout = fixture.bot("Scout");
    let ledger = fixture.bot("Ledger");

    assert!(!fixture.store.has_permission(&scout.id, &ledger.id).unwrap());
    let grant = fixture
        .store
        .grant_permission(&scout.id, &ledger.id)
        .unwrap();
    assert_eq!(
        (grant.from_bot_id.as_str(), grant.to_bot_id.as_str()),
        (scout.id.as_str(), ledger.id.as_str())
    );
    assert_eq!(grant.owner_id, LOCAL_OWNER_ID);
    assert_eq!(grant.granted_by, LOCAL_OWNER_ID);

    assert!(fixture.store.has_permission(&scout.id, &ledger.id).unwrap());
    assert!(!fixture.store.has_permission(&ledger.id, &scout.id).unwrap());
    assert_eq!(
        fixture
            .store
            .grant_permission(&scout.id, &ledger.id)
            .unwrap()
            .id,
        grant.id
    );
    fixture
        .store
        .grant_permission(&ledger.id, &scout.id)
        .unwrap();
    assert_eq!(fixture.store.permissions().unwrap().len(), 2);
}

#[test]
fn only_known_other_bots_can_be_granted() {
    let fixture = fixture();
    let scout = fixture.bot("Scout");

    for (from, to, code) in [
        (scout.id.as_str(), scout.id.as_str(), codes::INVALID),
        (scout.id.as_str(), "bot_nobody", codes::NOT_FOUND),
        ("bot_nobody", scout.id.as_str(), codes::NOT_FOUND),
    ] {
        assert_eq!(
            fixture.store.grant_permission(from, to).unwrap_err().code(),
            code
        );
    }
    assert!(fixture.store.permissions().unwrap().is_empty());
}

#[test]
fn revoking_and_deleting_a_bot_take_its_grants_back() {
    let fixture = fixture();
    let scout = fixture.bot("Scout");
    let ledger = fixture.bot("Ledger");
    let digest = fixture.bot("Digest");
    fixture
        .store
        .grant_permission(&scout.id, &ledger.id)
        .unwrap();
    fixture
        .store
        .grant_permission(&scout.id, &digest.id)
        .unwrap();
    fixture
        .store
        .grant_permission(&digest.id, &scout.id)
        .unwrap();
    fixture.changes.take();

    assert!(
        fixture
            .store
            .revoke_permission(&scout.id, &ledger.id)
            .unwrap()
    );
    assert!(
        !fixture
            .store
            .revoke_permission(&scout.id, &ledger.id)
            .unwrap()
    );
    assert!(!fixture.store.has_permission(&scout.id, &ledger.id).unwrap());
    assert_eq!(
        fixture.changes.take(),
        [
            Change::Bots(Some(scout.id.clone())),
            Change::Bots(Some(ledger.id.clone()))
        ]
    );

    fixture.store.delete_bot(&digest.id).unwrap();
    assert!(fixture.store.permissions().unwrap().is_empty());
}
