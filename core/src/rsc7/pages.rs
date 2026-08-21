const CLASSES: usize = 9;
const MULTIPLIERS: [u64; CLASSES] = [256, 128, 64, 32, 16, 8, 4, 2, 1];
const MAX_COUNTS: [u8; CLASSES] = [1, 3, 15, 63, 127, 1, 1, 1, 1];
const SHIFTS: [u32; CLASSES] = [4, 5, 7, 11, 17, 24, 25, 26, 27];
const MASKS: [u32; CLASSES] = [0x1, 0x3, 0xF, 0x3F, 0x7F, 0x1, 0x1, 0x1, 0x1];
const BASE_UNIT: u64 = 0x200;
const MAX_BASE_SHIFT: u32 = 15;
const MAX_PAGES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageLayout {
    base_shift: u32,
    counts: [u8; CLASSES],
}

impl PageLayout {
    pub fn from_flags(flags: u32) -> Self {
        let mut counts = [0u8; CLASSES];
        for (i, count) in counts.iter_mut().enumerate() {
            *count = ((flags >> SHIFTS[i]) & MASKS[i]) as u8;
        }
        Self {
            base_shift: flags & 0xF,
            counts,
        }
    }

    pub fn to_flags(&self, version_nibble: u32) -> u32 {
        let mut flags = ((version_nibble & 0xF) << 28) | (self.base_shift & 0xF);
        for (i, &count) in self.counts.iter().enumerate() {
            flags |= (u32::from(count) & MASKS[i]) << SHIFTS[i];
        }
        flags
    }

    pub fn base_size(&self) -> u64 {
        BASE_UNIT << self.base_shift
    }

    pub fn total_size(&self) -> u64 {
        let base = self.base_size();
        self.counts
            .iter()
            .zip(MULTIPLIERS)
            .map(|(&count, mult)| base * u64::from(count) * mult)
            .sum()
    }

    pub fn page_count(&self) -> usize {
        self.counts.iter().map(|&c| usize::from(c)).sum()
    }

    pub fn page_bounds(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        let base = self.base_size();
        let mut at = 0u64;
        self.counts
            .iter()
            .zip(MULTIPLIERS)
            .flat_map(move |(&count, mult)| std::iter::repeat_n(base * mult, usize::from(count)))
            .map(move |size| {
                let start = at;
                at += size;
                (start, at)
            })
    }

    pub fn page_of(&self, offset: u64) -> Option<usize> {
        self.page_bounds()
            .position(|(start, end)| (start..end).contains(&offset))
    }

    pub fn page_end(&self, offset: u64) -> Option<u64> {
        self.page_bounds()
            .find(|&(start, end)| (start..end).contains(&offset))
            .map(|(_, end)| end)
    }

    pub fn holds_block(&self, offset: u64, len: u64) -> bool {
        if len == 0 {
            return offset <= self.total_size();
        }
        match (self.page_of(offset), self.page_of(offset + len - 1)) {
            (Some(first), Some(last)) => first == last,
            _ => false,
        }
    }

    pub fn pack(block_sizes: &[usize], align: usize) -> Option<(Self, Vec<u64>)> {
        if block_sizes.iter().all(|&size| size == 0) {
            return Some((Self::default(), vec![0; block_sizes.len()]));
        }
        let align = (align.max(1) as u64).next_power_of_two();

        let mut order: Vec<usize> = (0..block_sizes.len())
            .filter(|&i| block_sizes[i] > 0)
            .collect();
        order.sort_unstable_by_key(|&i| std::cmp::Reverse(block_sizes[i]));
        let largest = block_sizes[order[0]] as u64;

        let mut best: Option<(Self, Vec<u64>)> = None;
        for base_shift in 0..=MAX_BASE_SHIFT {
            if (BASE_UNIT << base_shift) * MULTIPLIERS[0] < largest {
                continue;
            }
            for working in 0..CLASSES {
                let Some(candidate) = pack_with(block_sizes, &order, align, base_shift, working)
                else {
                    continue;
                };
                let better = best.as_ref().is_none_or(|(layout, _)| {
                    (candidate.0.total_size(), candidate.0.page_count())
                        < (layout.total_size(), layout.page_count())
                });
                if better {
                    best = Some(candidate);
                }
            }
        }
        best
    }
}

struct Page {
    class: usize,
    used: u64,
}

fn pack_with(
    sizes: &[usize],
    order: &[usize],
    align: u64,
    base_shift: u32,
    working: usize,
) -> Option<(PageLayout, Vec<u64>)> {
    let base = BASE_UNIT << base_shift;
    let capacity = |class: usize| base * MULTIPLIERS[class];

    let mut counts = [0u8; CLASSES];
    let mut pages: Vec<Page> = Vec::new();

    let mut placement = vec![(0usize, 0u64); sizes.len()];

    for &i in order {
        let size = sizes[i] as u64;

        let fitted = pages.iter_mut().enumerate().find_map(|(index, page)| {
            let at = page.used.next_multiple_of(align);
            (at + size <= capacity(page.class)).then(|| {
                page.used = at + size;
                (index, at)
            })
        });
        if let Some((index, at)) = fitted {
            placement[i] = (index, at);
            continue;
        }

        let class = (0..=working)
            .rev()
            .find(|&class| capacity(class) >= size && counts[class] < MAX_COUNTS[class])?;
        counts[class] += 1;
        pages.push(Page { class, used: size });
        placement[i] = (pages.len() - 1, 0);
    }

    shrink_pages(&mut pages, &mut counts, base);

    if pages.len() > MAX_PAGES {
        return None;
    }

    let mut by_class: Vec<usize> = (0..pages.len()).collect();
    by_class.sort_by_key(|&index| (pages[index].class, index));

    let mut starts = vec![0u64; pages.len()];
    let mut at = 0u64;
    for &index in &by_class {
        starts[index] = at;
        at += capacity(pages[index].class);
    }

    let offsets = (0..sizes.len())
        .map(|i| {
            let (page, within) = placement[i];
            starts.get(page).copied().unwrap_or(0) + within
        })
        .collect();

    Some((PageLayout { base_shift, counts }, offsets))
}

fn shrink_pages(pages: &mut [Page], counts: &mut [u8; CLASSES], base: u64) {
    let mut order: Vec<usize> = (0..pages.len()).collect();
    order.sort_by_key(|&index| std::cmp::Reverse(pages[index].used));

    for index in order {
        let (class, used) = (pages[index].class, pages[index].used);
        let smaller = (class + 1..CLASSES).find(|&smaller| {
            base * MULTIPLIERS[smaller] >= used && counts[smaller] < MAX_COUNTS[smaller]
        });
        if let Some(smaller) = smaller {
            counts[class] -= 1;
            counts[smaller] += 1;
            pages[index].class = smaller;
        }
    }
}

pub fn size_from_flags(flags: u32) -> u64 {
    PageLayout::from_flags(flags).total_size()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_flags_decode_to_their_real_page_runs() {
        let cases: [(u32, u64, &[u64]); 3] = [
            (0xd002_0007, 1_048_576, &[0x100000]),
            (0xd102_0008, 3_145_728, &[0x200000, 0x100000]),
            (
                0xd504_0008,
                5_505_024,
                &[0x200000, 0x200000, 0x100000, 0x40000],
            ),
        ];
        for (flags, size, run) in cases {
            let layout = PageLayout::from_flags(flags);
            assert_eq!(layout.total_size(), size, "{flags:#x}");
            let sizes: Vec<u64> = layout.page_bounds().map(|(a, b)| b - a).collect();
            assert_eq!(sizes, run, "{flags:#x}");
            assert_eq!(sizes.iter().sum::<u64>(), size, "{flags:#x}");
        }
    }

    #[test]
    fn flags_survive_a_round_trip_with_their_version_nibble() {
        for flags in [0xd002_0007u32, 0xd102_0008, 0xd504_0008, 0x7000_0000] {
            let layout = PageLayout::from_flags(flags);
            assert_eq!(layout.to_flags(flags >> 28), flags, "{flags:#x}");
        }
    }

    #[test]
    fn pages_are_ordered_largest_class_first() {
        let layout = PageLayout::from_flags(0xd504_0008);
        let sizes: Vec<u64> = layout.page_bounds().map(|(a, b)| b - a).collect();
        assert!(
            sizes.windows(2).all(|w| w[0] >= w[1]),
            "not descending: {sizes:?}"
        );
    }

    fn assert_packs_cleanly(blocks: &[usize]) -> PageLayout {
        let (layout, offsets) = PageLayout::pack(blocks, 16).expect("packable");
        let mut spans: Vec<(u64, u64)> = Vec::new();
        for (&size, &offset) in blocks.iter().zip(&offsets) {
            let size = size as u64;
            assert!(
                layout.holds_block(offset, size),
                "block of {size:#x} at {offset:#x} crosses a page in {layout:?}"
            );
            assert!(
                offset + size <= layout.total_size(),
                "block of {size:#x} at {offset:#x} runs past the segment"
            );
            assert!(offset.is_multiple_of(16), "{offset:#x} is not 16 aligned");
            if size > 0 {
                spans.push((offset, offset + size));
            }
        }
        spans.sort_unstable();
        for pair in spans.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "blocks overlap: {:?} and {:?}",
                pair[0],
                pair[1]
            );
        }
        assert!(layout.page_count() <= MAX_PAGES);
        layout
    }

    #[test]
    fn a_packed_layout_never_straddles_overlaps_or_overruns() {
        let cases: Vec<Vec<usize>> = vec![
            vec![0x1000],
            vec![0xf4240],
            vec![0x1fa400, 0x1fa400, 0x14a0],
            vec![0x7900; 40],
            (1..=64).map(|n| n * 0x400).collect(),
            vec![0x155_5540, 0x2b_0000, 0x4, 0x10],
            vec![0x20, 0x20, 0x20, 0x20],
        ];
        for blocks in cases {
            assert_packs_cleanly(&blocks);
        }
    }

    #[test]
    fn packing_a_real_dictionary_beats_or_matches_the_original() {
        let mut blocks = vec![0x1fa400, 0x1fa400];
        blocks.extend(std::iter::repeat_n(0x7900, 34));
        blocks.extend(std::iter::repeat_n(0x7bc0, 6));
        blocks.extend([0x5540, 0x5540, 0x14a0]);

        let layout = assert_packs_cleanly(&blocks);
        let shipped = PageLayout::from_flags(0xd504_0008).total_size();
        assert!(
            layout.total_size() <= shipped,
            "packed to {:#x}, the file shipped {shipped:#x}",
            layout.total_size()
        );
        let payload: usize = blocks.iter().sum();
        assert!(
            layout.total_size() >= payload as u64,
            "layout does not cover its own payload"
        );
    }

    #[test]
    fn every_offset_lands_inside_a_page() {
        let blocks: Vec<usize> = (0..200).map(|n| 0x800 + n * 0x40).collect();
        let (layout, offsets) = PageLayout::pack(&blocks, 16).expect("packable");
        for &offset in &offsets {
            assert!(layout.page_of(offset).is_some(), "{offset:#x} is homeless");
        }
    }

    #[test]
    fn zero_sized_blocks_pack_to_an_empty_layout() {
        let (layout, offsets) = PageLayout::pack(&[0, 0, 0], 16).expect("packable");
        assert_eq!(layout.total_size(), 0);
        assert_eq!(layout.page_count(), 0);
        assert_eq!(offsets, vec![0, 0, 0]);
    }

    #[test]
    fn an_unpackable_block_is_refused_rather_than_split() {
        let too_big = ((BASE_UNIT << MAX_BASE_SHIFT) * MULTIPLIERS[0] + 1) as usize;
        assert!(PageLayout::pack(&[too_big], 16).is_none());
    }

    #[test]
    fn class_budgets_are_never_exceeded() {
        let blocks: Vec<usize> = std::iter::repeat_n(0x1000, 500).collect();
        let (layout, _) = PageLayout::pack(&blocks, 16).expect("packable");
        for (i, &count) in layout.counts.iter().enumerate() {
            assert!(
                count <= MAX_COUNTS[i],
                "class {i} holds {count}, the flags word tops out at {}",
                MAX_COUNTS[i]
            );
        }
        assert_eq!(layout, PageLayout::from_flags(layout.to_flags(0xd)));
    }

    #[test]
    fn a_realistic_dictionary_packs_close_to_its_payload() {
        let mut blocks: Vec<usize> = Vec::new();
        blocks.extend(std::iter::repeat_n(0x55550, 10));
        blocks.extend(std::iter::repeat_n(0x2aaa8, 12));
        blocks.extend(std::iter::repeat_n(0x15550, 20));
        blocks.extend(std::iter::repeat_n(0xaaa8, 25));
        blocks.extend(std::iter::repeat_n(0x1550, 20));
        blocks.extend(std::iter::repeat_n(0x10, 25));

        let payload: usize = blocks.iter().sum();
        let layout = assert_packs_cleanly(&blocks);
        let waste = layout.total_size() as f64 / payload as f64;
        assert!(
            waste < 1.5,
            "packed {} blocks totalling {payload:#x} into {:#x}, {waste:.2}x the payload",
            blocks.len(),
            layout.total_size()
        );
    }

    #[test]
    fn a_page_end_bounds_what_can_start_there() {
        let layout = PageLayout::from_flags(0xd504_0008);
        assert_eq!(layout.page_end(0), Some(0x200000));
        assert_eq!(layout.page_end(0x1fffff), Some(0x200000));
        assert_eq!(layout.page_end(0x200000), Some(0x400000));
        assert_eq!(layout.page_end(0x520000), Some(0x540000));
        assert_eq!(layout.page_end(layout.total_size()), None);
    }

    #[test]
    fn holds_block_catches_a_straddle() {
        let layout = PageLayout::from_flags(0xd504_0008);
        assert!(layout.holds_block(0, 0x200000));
        assert!(layout.holds_block(0x1fff00, 0x100));
        assert!(
            !layout.holds_block(0x1fff00, 0x200),
            "straddle went unnoticed"
        );
        assert!(!layout.holds_block(layout.total_size(), 1), "past the end");
    }
}
