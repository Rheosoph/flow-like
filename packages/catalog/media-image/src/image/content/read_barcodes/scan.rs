//! Every symbol in one grayscale image. Matrix codes go through rxing's detectors; linear codes
//! are read row by row, and the rows that agree become one symbol boxed around its bars.

use std::sync::LazyLock;

use rxing::{
    BarcodeFormat, BinaryBitmap, DecodeHints, Luma8LuminanceSource, MultiFormatReader, RXingResult,
    Reader,
    aztec::AztecReader,
    common::{BitArray, DetectorRXingResult, HybridBinarizer},
    datamatrix::{DataMatrixReader, decoder::Decoder, detector::zxing_cpp_detector},
    maxicode::MaxiCodeReader,
    multi::MultipleBarcodeReader,
    oned::{MultiFormatOneDReader, OneDReader},
    pdf417::PDF417Reader,
    point_f,
    qrcode::{QRCodeReader, cpp_port::QrReader},
};

/// Pixels thresholded together along a row.
const BLOCK: usize = 8;
/// Blocks on either side that share one threshold.
const NEIGHBOURS: usize = 2;
/// Less spread than this within a neighbourhood is one flat colour.
const MIN_CONTRAST: i32 = 24;
/// Blocks a flat stretch may lie from an edge and still take its threshold, so wide bars stay
/// dark inside.
const FLAT_REACH: usize = 12;
const MAX_SYMBOLS_PER_ROW: usize = 6;
/// Rows at the usual spacing a symbol may fail to decode on between two rows that agree.
const MAX_MISSED_ROWS: usize = 1;
/// Share of the pixels of a symbol's middle row another row has to repeat to show its bars.
const SAME_BARS: f32 = 0.85;

static DATA_MATRIX: LazyLock<Decoder> = LazyLock::new(Decoder::new);

/// An axis-aligned box in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Bounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Bounds {
    pub fn around(points: impl IntoIterator<Item = (f32, f32)>) -> Option<Self> {
        let mut points = points.into_iter();
        let (x, y) = points.next()?;
        let mut bounds = Self {
            left: x,
            top: y,
            right: x,
            bottom: y,
        };
        for (x, y) in points {
            bounds.left = bounds.left.min(x);
            bounds.right = bounds.right.max(x);
            bounds.top = bounds.top.min(y);
            bounds.bottom = bounds.bottom.max(y);
        }
        Some(bounds)
    }

    pub fn of(result: &RXingResult) -> Option<Self> {
        Self::around(result.getPoints().iter().map(|point| (point.x, point.y)))
    }

    fn area(&self) -> f32 {
        (self.right - self.left + 1.0) * (self.bottom - self.top + 1.0)
    }

    fn shared_area(&self, other: &Self) -> f32 {
        let width = (self.right.min(other.right) - self.left.max(other.left) + 1.0).max(0.0);
        let height = (self.bottom.min(other.bottom) - self.top.max(other.top) + 1.0).max(0.0);
        width * height
    }

    /// More than half of the smaller box lies in the other. Two printed symbols never share
    /// that much, so both boxes show the same one.
    pub fn same_place(&self, other: &Self) -> bool {
        self.shared_area(other) > 0.5 * self.area().min(other.area())
    }

    fn mostly_inside(&self, other: &Self) -> bool {
        self.shared_area(other) > 0.5 * self.area()
    }
}

pub(super) fn is_linear(format: BarcodeFormat) -> bool {
    matches!(
        format,
        BarcodeFormat::CODABAR
            | BarcodeFormat::CODE_39
            | BarcodeFormat::CODE_93
            | BarcodeFormat::CODE_128
            | BarcodeFormat::EAN_8
            | BarcodeFormat::EAN_13
            | BarcodeFormat::ITF
            | BarcodeFormat::RSS_14
            | BarcodeFormat::RSS_EXPANDED
            | BarcodeFormat::TELEPEN
            | BarcodeFormat::UPC_A
            | BarcodeFormat::UPC_E
    )
}

/// Rows that have to agree before a linear symbol counts. Formats without a check digit, and the
/// short ones text imitates, need one more.
fn required_rows(format: BarcodeFormat) -> usize {
    match format {
        BarcodeFormat::CODABAR
        | BarcodeFormat::CODE_39
        | BarcodeFormat::EAN_8
        | BarcodeFormat::ITF
        | BarcodeFormat::TELEPEN
        | BarcodeFormat::UPC_E => 3,
        _ => 2,
    }
}

/// Distance between the rows read for linear codes: about 135 of them across the short side.
fn row_spacing(width: usize, height: usize) -> usize {
    (width.min(height) / 135).clamp(1, 8)
}

fn hints_for(hints: &DecodeHints, formats: &[BarcodeFormat]) -> DecodeHints {
    let mut hints = hints.clone();
    hints.PossibleFormats = Some(formats.iter().copied().collect());
    hints
}

struct LinearReader {
    reader: MultiFormatOneDReader,
    hints: DecodeHints,
}

/// Reads the symbols of the chosen formats, one image after the other.
pub(super) struct Scanner {
    hints: DecodeHints,
    formats: Vec<BarcodeFormat>,
    linear: Option<LinearReader>,
    pure: Option<MultiFormatReader>,
    try_harder: bool,
}

impl Scanner {
    /// `hints.AlsoInverted` is ignored: hand the inverted pixels to [`Scanner::scan`] instead,
    /// which reads linear codes of that polarity too.
    pub fn new(hints: &DecodeHints, formats: &[BarcodeFormat]) -> Self {
        let mut hints = hints_for(hints, formats);
        hints.AlsoInverted = Some(false);
        let linear_formats: Vec<_> = formats
            .iter()
            .copied()
            .filter(|format| is_linear(*format))
            .collect();
        let linear = (!linear_formats.is_empty()).then(|| {
            let hints = hints_for(&hints, &linear_formats);
            LinearReader {
                reader: MultiFormatOneDReader::new(&hints),
                hints,
            }
        });
        let pure = (hints.PureBarcode == Some(true)).then(|| {
            let mut reader = MultiFormatReader::default();
            reader.set_hints(&hints);
            reader
        });
        Self {
            try_harder: hints.TryHarder == Some(true),
            hints,
            formats: formats.to_vec(),
            linear,
            pure,
        }
    }

    /// The symbols in `luma` (`width * height` bytes). Linear symbols come with the four
    /// corners of their bars and the number of rows that read them.
    pub fn scan(&mut self, luma: &[u8], width: u32, height: u32) -> Vec<RXingResult> {
        if width == 0 || height == 0 || luma.len() != width as usize * height as usize {
            return Vec::new();
        }
        if let Some(reader) = &mut self.pure {
            let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(Luma8LuminanceSource::new(
                luma.to_vec(),
                width,
                height,
            )));
            reader.reset();
            return reader.decode_with_state(&mut bitmap).into_iter().collect();
        }

        let linear = match &mut self.linear {
            Some(linear) => linear_symbols(
                luma,
                width as usize,
                height as usize,
                linear,
                self.try_harder,
            ),
            None => Vec::new(),
        };
        let mut found = self.matrix_symbols(luma, width, height);
        let matrix_boxes: Vec<_> = found.iter().filter_map(Bounds::of).collect();
        // Rows through a matrix code sometimes pass for a short linear one.
        found.extend(linear.into_iter().filter(|symbol| {
            Bounds::of(symbol).is_none_or(|bounds| {
                !matrix_boxes
                    .iter()
                    .any(|matrix| bounds.mostly_inside(matrix))
            })
        }));
        found
    }

    fn matrix_symbols(&self, luma: &[u8], width: u32, height: u32) -> Vec<RXingResult> {
        let detectors: Vec<_> = MatrixDetector::ALL
            .into_iter()
            .filter(|detector| self.formats.iter().any(|format| detector.reads(*format)))
            .collect();
        if detectors.is_empty() {
            return Vec::new();
        }
        let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(Luma8LuminanceSource::new(
            luma.to_vec(),
            width,
            height,
        )));
        let mut found: Vec<RXingResult> = Vec::new();
        for detector in detectors {
            for symbol in detector.symbols(&mut bitmap, &self.hints) {
                // A detector that traces from several sides meets one symbol more than once.
                if !found.iter().any(|known| same_symbol(known, &symbol)) {
                    found.push(symbol);
                }
            }
        }
        found
    }
}

fn same_symbol(a: &RXingResult, b: &RXingResult) -> bool {
    a.getBarcodeFormat() == b.getBarcodeFormat()
        && a.getText() == b.getText()
        && match (Bounds::of(a), Bounds::of(b)) {
            (Some(a), Some(b)) => a.same_place(&b),
            _ => true,
        }
}

type Bitmap = BinaryBitmap<HybridBinarizer<Luma8LuminanceSource>>;

#[derive(Clone, Copy)]
enum MatrixDetector {
    Qr,
    DataMatrix,
    Pdf417,
    Aztec,
    MaxiCode,
}

impl MatrixDetector {
    const ALL: [Self; 5] = [
        Self::Qr,
        Self::DataMatrix,
        Self::Pdf417,
        Self::Aztec,
        Self::MaxiCode,
    ];

    fn reads(self, format: BarcodeFormat) -> bool {
        match self {
            Self::Qr => matches!(
                format,
                BarcodeFormat::QR_CODE
                    | BarcodeFormat::MICRO_QR_CODE
                    | BarcodeFormat::RECTANGULAR_MICRO_QR_CODE
            ),
            Self::DataMatrix => format == BarcodeFormat::DATA_MATRIX,
            Self::Pdf417 => format == BarcodeFormat::PDF_417,
            Self::Aztec => format == BarcodeFormat::AZTEC,
            Self::MaxiCode => format == BarcodeFormat::MAXICODE,
        }
    }

    /// Every symbol the detector finds; Aztec and MaxiCode look around the centre only.
    fn symbols(self, bitmap: &mut Bitmap, hints: &DecodeHints) -> Vec<RXingResult> {
        match self {
            Self::Qr => qr_symbols(bitmap, hints),
            Self::DataMatrix => data_matrix_symbols(bitmap, hints),
            Self::Pdf417 => PDF417Reader
                .decode_multiple_with_hints(bitmap, hints)
                .unwrap_or_default(),
            Self::Aztec => AztecReader
                .decode_with_hints(bitmap, hints)
                .into_iter()
                .collect(),
            Self::MaxiCode => MaxiCodeReader::default()
                .decode_with_hints(bitmap, hints)
                .into_iter()
                .collect(),
        }
    }
}

/// One pass finds every QR family the hints enable; the older reader gets a second look at an
/// image the first found nothing in.
fn qr_symbols(bitmap: &mut Bitmap, hints: &DecodeHints) -> Vec<RXingResult> {
    let found = QrReader
        .decode_multiple_with_hints(bitmap, hints)
        .unwrap_or_default();
    let standard = hints
        .PossibleFormats
        .as_ref()
        .is_some_and(|formats| formats.contains(&BarcodeFormat::QR_CODE));
    if !found.is_empty() || !standard {
        return found;
    }
    QRCodeReader
        .decode_with_hints(bitmap, hints)
        .into_iter()
        .collect()
}

/// rxing's reader stops at the first Data Matrix on the image's centre lines. The detector
/// underneath traces the whole image and hands back every symbol.
fn data_matrix_symbols(bitmap: &mut Bitmap, hints: &DecodeHints) -> Vec<RXingResult> {
    let mut found = Vec::new();
    for symbol in
        zxing_cpp_detector::detect(bitmap.get_black_matrix(), true, true).unwrap_or_default()
    {
        if let Ok(decoded) = DATA_MATRIX.decode(symbol.getBits()) {
            found.push(RXingResult::new(
                decoded.getText(),
                decoded.getRawBytes().clone(),
                symbol.getPoints().to_vec(),
                BarcodeFormat::DATA_MATRIX,
            ));
        }
    }
    if found.is_empty()
        && let Ok(symbol) = DataMatrixReader.decode_with_hints(bitmap, hints)
    {
        found.push(symbol);
    }
    found
}

/// Thresholds the rows of an image, each averaged with its neighbours: noise drops while the
/// edges of upright bars stay where they are.
struct Rows {
    width: usize,
    height: usize,
    radius: usize,
    sums: Vec<i32>,
    ranges: Vec<(i32, i32)>,
    edges: Vec<Option<i32>>,
    thresholds: Vec<Option<i32>>,
}

impl Rows {
    fn new(width: usize, height: usize, radius: usize) -> Self {
        let blocks = width.div_ceil(BLOCK);
        Self {
            width,
            height,
            radius,
            sums: vec![0; width],
            ranges: vec![(0, 0); blocks],
            edges: vec![None; blocks],
            thresholds: vec![None; blocks],
        }
    }

    /// Dark pixels of row `y` of `luma`, thresholded against their surroundings so a shadow or
    /// a dark table next to the paper does not swallow the bars.
    fn read(&mut self, luma: &[u8], y: usize) -> BitArray {
        let from = y.saturating_sub(self.radius);
        let to = (y + self.radius).min(self.height - 1);
        self.sums.fill(0);
        for row in luma[from * self.width..(to + 1) * self.width].chunks_exact(self.width) {
            for (sum, pixel) in self.sums.iter_mut().zip(row) {
                *sum += i32::from(*pixel);
            }
        }
        self.set_thresholds(MIN_CONTRAST * (to - from + 1) as i32);

        let mut bits = BitArray::with_size(self.width);
        for x in 0..self.width {
            if self.thresholds[x / BLOCK].is_some_and(|threshold| self.sharpened(x) < threshold) {
                bits.set(x);
            }
        }
        bits
    }

    /// The -1 4 -1 filter rxing sharpens rows with; it keeps one-pixel bars apart.
    fn sharpened(&self, x: usize) -> i32 {
        if x == 0 || x + 1 == self.width {
            return self.sums[x];
        }
        (4 * self.sums[x] - self.sums[x - 1] - self.sums[x + 1]) / 2
    }

    /// Half way between the darkest and the lightest sum around each block. A stretch without
    /// `min_contrast` takes the threshold of the nearest edge, or none when that is far.
    fn set_thresholds(&mut self, min_contrast: i32) {
        for (range, block) in self.ranges.iter_mut().zip(self.sums.chunks(BLOCK)) {
            let low = block.iter().copied().min().unwrap_or(0);
            let high = block.iter().copied().max().unwrap_or(0);
            *range = (low, high);
        }
        let blocks = self.ranges.len();
        for block in 0..blocks {
            let near = &self.ranges
                [block.saturating_sub(NEIGHBOURS)..(block + NEIGHBOURS + 1).min(blocks)];
            let low = near.iter().map(|range| range.0).min().unwrap_or(0);
            let high = near.iter().map(|range| range.1).max().unwrap_or(0);
            self.edges[block] = (high - low >= min_contrast).then_some((low + high) / 2);
        }
        self.thresholds.copy_from_slice(&self.edges);
        self.spread_thresholds(0..blocks);
        self.spread_thresholds((0..blocks).rev());
    }

    fn spread_thresholds(&mut self, order: impl Iterator<Item = usize>) {
        let mut carried: Option<(i32, usize)> = None;
        for block in order {
            if let Some(threshold) = self.edges[block] {
                carried = Some((threshold, 0));
                continue;
            }
            let Some((threshold, distance)) = &mut carried else {
                continue;
            };
            *distance += 1;
            if *distance <= FLAT_REACH && self.thresholds[block].is_none() {
                self.thresholds[block] = Some(*threshold);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Edge {
    y: usize,
    left: usize,
    right: usize,
}

#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
}

struct LinearSymbol {
    result: RXingResult,
    rows: usize,
    top: Edge,
    bottom: Edge,
}

impl LinearSymbol {
    fn bounds(&self) -> Bounds {
        Bounds {
            left: self.top.left.min(self.bottom.left) as f32,
            right: self.top.right.max(self.bottom.right) as f32,
            top: self.top.y as f32,
            bottom: self.bottom.y as f32,
        }
    }

    fn same_text(&self, result: &RXingResult) -> bool {
        self.result.getBarcodeFormat() == result.getBarcodeFormat()
            && self.result.getText() == result.getText()
    }

    /// `result`, read between the ends of `edge`, is a further row of this symbol: below its
    /// last one, at most `reach` pixels away.
    fn continues_in(&self, result: &RXingResult, edge: &Edge, reach: usize) -> bool {
        let shared =
            self.bottom.right.min(edge.right) as isize - self.bottom.left.max(edge.left) as isize;
        let narrower = (self.bottom.right - self.bottom.left).min(edge.right - edge.left);
        self.same_text(result)
            && edge.y > self.bottom.y
            && edge.y - self.bottom.y <= reach
            && shared * 2 > narrower as isize
    }

    /// Enough rows agree, over enough of the image. Rows read closer together than usual
    /// share most of their pixels, so they have to span what the usual ones would.
    fn is_confirmed(&self, spacing: usize, row_count: usize) -> bool {
        let required = required_rows(*self.result.getBarcodeFormat()).min(row_count);
        self.rows >= required && self.bottom.y - self.top.y >= (required - 1) * spacing
    }

    /// The ends of the bars on row `y`, between those of the outermost rows that read.
    fn edge_at(&self, y: usize) -> Edge {
        let span = (self.bottom.y - self.top.y).max(1) as isize;
        let done = (y - self.top.y) as isize;
        let between = |top: usize, bottom: usize| {
            (top as isize + (bottom as isize - top as isize) * done / span) as usize
        };
        Edge {
            y,
            left: between(self.top.left, self.bottom.left),
            right: between(self.top.right, self.bottom.right),
        }
    }

    /// Sets top and bottom to where the ink of the bars ends. The outermost rows that read
    /// may lie past it, reached only through the averaging, so the search starts in the middle.
    fn box_bars(&mut self, rows: &mut BarRows, luma: &[u8]) {
        let middle = self.edge_at((self.top.y + self.bottom.y) / 2);
        // Fading bars agree with the blank paper after them; the ink has to stay.
        let least_dark = dark_pixels(&rows.single.read(luma, middle.y), &middle, 0) * 7 / 10;
        if least_dark == 0 {
            return;
        }
        self.top = follow_bars(rows, luma, middle, least_dark, Direction::Up);
        self.bottom = follow_bars(rows, luma, middle, least_dark, Direction::Down);
    }

    fn finish(mut self) -> RXingResult {
        let corner = |x: usize, y: usize| point_f(x as f32, y as f32);
        self.result.replace_points(vec![
            corner(self.top.left, self.top.y),
            corner(self.top.right, self.top.y),
            corner(self.bottom.right, self.bottom.y),
            corner(self.bottom.left, self.bottom.y),
        ]);
        self.result.set_line_count(self.rows);
        self.result
    }
}

/// The two readings of a row that boxing a symbol takes.
struct BarRows {
    /// Averaged with the rows around, as the symbol was decoded: whether the bars go on.
    averaged: Rows,
    /// The row alone. Averaging carries the bars past their ends; this stops where the ink does.
    single: Rows,
}

/// Follows the bars from `origin` up or down, pixel row by pixel row, to the last row that
/// still shows them. A tilted symbol drifts sideways on the way, so each row is compared one
/// pixel left and right of where the row before it matched.
fn follow_bars(
    rows: &mut BarRows,
    luma: &[u8],
    origin: Edge,
    least_dark: usize,
    direction: Direction,
) -> Edge {
    let (width, height) = (rows.single.width as isize, rows.single.height);
    let reference = rows.averaged.read(luma, origin.y);
    let (mut y, mut drift) = (origin.y, 0isize);
    loop {
        let next_y = match direction {
            Direction::Up => y.checked_sub(1),
            Direction::Down => Some(y + 1).filter(|y| *y < height),
        };
        let Some(next_y) = next_y else { break };
        let next = rows.averaged.read(luma, next_y);
        let mut best: Option<(f32, isize)> = None;
        for shift in [drift, drift - 1, drift + 1] {
            if origin.left as isize + shift < 0 || origin.right as isize + shift >= width {
                continue;
            }
            let share = agreement(&reference, &next, &origin, shift);
            if best.is_none_or(|(most, _)| share > most) {
                best = Some((share, shift));
            }
        }
        let Some((share, shift)) = best else { break };
        let ink = dark_pixels(&rows.single.read(luma, next_y), &origin, shift);
        if share < SAME_BARS || ink < least_dark {
            break;
        }
        (y, drift) = (next_y, shift);
    }
    Edge {
        y,
        left: (origin.left as isize + drift) as usize,
        right: (origin.right as isize + drift) as usize,
    }
}

/// Share of the pixels between the edge's ends that `next`, moved by `shift`, repeats.
fn agreement(reference: &BitArray, next: &BitArray, edge: &Edge, shift: isize) -> f32 {
    let same = (edge.left..=edge.right)
        .filter(|x| reference.get(*x) == next.get((*x as isize + shift) as usize))
        .count();
    same as f32 / (edge.right - edge.left + 1) as f32
}

fn dark_pixels(row: &BitArray, edge: &Edge, shift: isize) -> usize {
    (edge.left..=edge.right)
        .filter(|x| row.get((*x as isize + shift) as usize))
        .count()
}

/// Runs of equal pixels a module width is judged from.
const SAMPLED_RUNS: usize = 18;
/// Modules of white that end a symbol. Inside one the widest space has four, a little more
/// where print or blur thins the bars; the quiet zone around it has seven or more.
const QUIET_MODULES: usize = 6;

/// The width of a module of the symbol that covers pixel `from`, judged from the runs of equal
/// pixels to its right. Narrow bars and spaces are the most common, so the lower third is one.
fn module_width(bits: &BitArray, from: usize) -> usize {
    let mut runs = Vec::with_capacity(SAMPLED_RUNS);
    let mut start = from;
    for x in from + 1..bits.get_size() {
        if bits.get(x) != bits.get(start) {
            runs.push(x - start);
            start = x;
            if runs.len() == SAMPLED_RUNS {
                break;
            }
        }
    }
    runs.sort_unstable();
    runs.get(runs.len() / 3).copied().unwrap_or(1)
}

/// The last dark pixel of the symbol `xs` leaves from `from`. The symbol ends before the first
/// run wider than `gap`: its quiet zone, or a shadow that thresholding turned solid.
fn last_bar(bits: &BitArray, from: usize, xs: impl Iterator<Item = usize>, gap: usize) -> usize {
    let (mut last, mut run_start, mut previous) = (from, from, from);
    for x in xs {
        if bits.get(x) != bits.get(previous) {
            if bits.get(previous) {
                last = previous;
            }
            run_start = x;
        } else if x.abs_diff(run_start) > gap {
            return last;
        }
        previous = x;
    }
    // The search ended on a bar.
    if bits.get(previous) { previous } else { last }
}

/// Modules the bars reach past the two points rxing reports for a symbol. Those are the centres
/// of the start and stop patterns; for DataBar they are the finder patterns further inside.
fn modules_past_points(format: BarcodeFormat) -> usize {
    match format {
        BarcodeFormat::RSS_14 | BarcodeFormat::RSS_EXPANDED => 26,
        _ => 8,
    }
}

/// Widest a Code 39 character gets, in modules, with the gap that follows it.
const CODE_39_CHARACTER: usize = 17;

fn linear_symbols(
    luma: &[u8],
    width: usize,
    height: usize,
    linear: &mut LinearReader,
    try_harder: bool,
) -> Vec<RXingResult> {
    let spacing = row_spacing(width, height);
    // Trying harder reads four times the rows. What a symbol needs stays measured in pixels.
    let step = if try_harder {
        spacing.div_ceil(4)
    } else {
        spacing
    };
    let reach = (MAX_MISSED_ROWS + 1) * spacing;
    let mut rows = BarRows {
        averaged: Rows::new(width, height, (spacing / 2).min(2)),
        single: Rows::new(width, height, 0),
    };
    let mut symbols: Vec<LinearSymbol> = Vec::new();
    linear.reader.reset();
    let mut row_count = 0;
    // The rows read when trying harder include the usual ones, so it never reads less.
    for y in ((spacing / 2) % step..height).step_by(step) {
        row_count += 1;
        for (result, edge) in read_row(linear, rows.averaged.read(luma, y), y) {
            match symbols
                .iter_mut()
                .find(|symbol| symbol.continues_in(&result, &edge, reach))
            {
                Some(symbol) => {
                    symbol.rows += 1;
                    symbol.bottom = edge;
                }
                None => symbols.push(LinearSymbol {
                    result,
                    rows: 1,
                    top: edge,
                    bottom: edge,
                }),
            }
        }
    }

    symbols.retain(|symbol| symbol.is_confirmed(spacing, row_count));
    for symbol in &mut symbols {
        symbol.box_bars(&mut rows, luma);
    }
    settle(symbols)
        .into_iter()
        .map(LinearSymbol::finish)
        .collect()
}

/// Every symbol one row holds, read in both directions, with the ends of its bars.
fn read_row(linear: &mut LinearReader, mut bits: BitArray, y: usize) -> Vec<(RXingResult, Edge)> {
    let width = bits.get_size();
    let mut found = Vec::new();
    for reversed in [false, true] {
        if reversed {
            bits.reverse();
        }
        for _ in 0..MAX_SYMBOLS_PER_ROW {
            let Ok(result) = linear.reader.decode_row(y as u32, &bits, &linear.hints) else {
                break;
            };
            let Some((from, to)) = symbol_span(&result, &bits) else {
                break;
            };
            // Blank the symbol: the next pass reads what else the row holds.
            // `BitArray::unset` is a no-op in rxing 0.7.1.
            for x in from..=to {
                if bits.get(x) {
                    bits.flip(x);
                }
            }
            let (left, right) = if reversed {
                (width - 1 - to, width - 1 - from)
            } else {
                (from, to)
            };
            found.push((result, Edge { y, left, right }));
        }
    }
    found
}

/// Where the bars of a symbol decoded from `bits` begin and end: from the points rxing reports
/// outwards to the quiet zones, no further than the symbol can reach.
fn symbol_span(result: &RXingResult, bits: &BitArray) -> Option<(usize, usize)> {
    let [start, end, ..] = result.getPoints() else {
        return None;
    };
    let size = bits.get_size();
    let from = (start.x.max(0.0) as usize).min(size - 1);
    let module = module_width(bits, from);
    let gap = QUIET_MODULES * module + 1;
    let format = *result.getBarcodeFormat();
    let past = modules_past_points(format) * module;
    // rxing 0.7.1 halves the end of a Code 39 symbol, which can put it before the start.
    // That one is followed from its start, as far as its characters can take it.
    let (to, past_end) = if format == BarcodeFormat::CODE_39 {
        let characters = 2 * result.getText().len() + 3;
        (from, characters * CODE_39_CHARACTER * module)
    } else {
        ((end.x.max(start.x) as usize).min(size - 1), past)
    };
    let left = last_bar(bits, from, (from.saturating_sub(past)..from).rev(), gap);
    let right = last_bar(bits, to, to + 1..(to + past_end + 1).min(size), gap);
    (right > left).then_some((left, right))
}

/// One run of bars read as several symbols: rows that failed in between split it, or some rows
/// read only part of it. The reading most rows agree on stays, the longer one on a tie.
fn settle(mut symbols: Vec<LinearSymbol>) -> Vec<LinearSymbol> {
    symbols.sort_by(|a, b| {
        (b.rows, b.result.getText().len()).cmp(&(a.rows, a.result.getText().len()))
    });
    let mut kept: Vec<LinearSymbol> = Vec::new();
    for symbol in symbols {
        let bounds = symbol.bounds();
        let Some(other) = kept
            .iter_mut()
            .find(|other| other.bounds().same_place(&bounds))
        else {
            kept.push(symbol);
            continue;
        };
        if other.same_text(&symbol.result) {
            other.rows += symbol.rows;
            if symbol.top.y < other.top.y {
                other.top = symbol.top;
            }
            if symbol.bottom.y > other.bottom.y {
                other.bottom = symbol.bottom;
            }
        }
    }
    kept
}
