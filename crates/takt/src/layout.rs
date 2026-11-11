//! Where overlapping events go: side-by-side columns in a time grid, and
//! stacked lanes for bars that run across days. Pure: no GPUI types.

/// One timed item's extent on a day, in minutes from local midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    fn overlaps(&self, other: &Span) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// An item's place among the ones it overlaps: `column` of `columns`, wide
/// enough to cover `span` of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub column: usize,
    pub columns: usize,
    pub span: usize,
}

/// Lays out one day's timed items the way Google Calendar does. Items that
/// overlap, directly or through others, form a cluster and share its width
/// equally; each takes the leftmost free column, and then widens to the right
/// over columns it has to itself. The result is in input order.
pub fn pack_columns(spans: &[Span]) -> Vec<Slot> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    // Earlier first; among equal starts the longer one goes left.
    order.sort_by_key(|&i| (spans[i].start, std::cmp::Reverse(spans[i].end), i));

    let mut slots = vec![
        Slot {
            column: 0,
            columns: 1,
            span: 1,
        };
        spans.len()
    ];
    let mut cluster: Vec<usize> = Vec::new();
    // The end of the last item in each column of the open cluster.
    let mut column_ends: Vec<u32> = Vec::new();
    let mut cluster_end = 0;

    for &i in &order {
        if !cluster.is_empty() && spans[i].start >= cluster_end {
            close_cluster(&cluster, column_ends.len(), spans, &mut slots);
            cluster.clear();
            column_ends.clear();
        }
        let column = column_ends
            .iter()
            .position(|&end| end <= spans[i].start)
            .unwrap_or(column_ends.len());
        if column == column_ends.len() {
            column_ends.push(spans[i].end);
        } else {
            column_ends[column] = spans[i].end;
        }
        slots[i].column = column;
        cluster_end = if cluster.is_empty() {
            spans[i].end
        } else {
            cluster_end.max(spans[i].end)
        };
        cluster.push(i);
    }
    close_cluster(&cluster, column_ends.len(), spans, &mut slots);
    slots
}

fn close_cluster(cluster: &[usize], columns: usize, spans: &[Span], slots: &mut [Slot]) {
    for &i in cluster {
        let free = |column: usize| {
            !cluster
                .iter()
                .any(|&j| j != i && slots[j].column == column && spans[j].overlaps(&spans[i]))
        };
        let span = 1
            + (slots[i].column + 1..columns)
                .take_while(|&c| free(c))
                .count();
        slots[i].columns = columns;
        slots[i].span = span;
    }
}

/// A bar over the day columns `first..first + len` of one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarSpan {
    pub first: usize,
    pub len: usize,
}

impl BarSpan {
    fn covers(&self, column: usize) -> bool {
        (self.first..self.first + self.len).contains(&column)
    }
}

/// Gives each bar the topmost lane that is free on all its days. Bars are
/// placed in input order, so the caller sorts the ones that should sit higher
/// first. Returns one lane per bar.
pub fn pack_lanes(bars: &[BarSpan], columns: usize) -> Vec<usize> {
    // lanes[l][c]: lane `l` is taken on column `c`.
    let mut lanes: Vec<Vec<bool>> = Vec::new();
    bars.iter()
        .map(|bar| {
            let cells = bar.first.min(columns)..(bar.first + bar.len).min(columns);
            let lane = lanes
                .iter()
                .position(|taken| !taken[cells.clone()].iter().any(|t| *t))
                .unwrap_or_else(|| {
                    lanes.push(vec![false; columns]);
                    lanes.len() - 1
                });
            lanes[lane][cells].fill(true);
            lane
        })
        .collect()
}

/// What a row with room for `cap` lanes shows: which bars, and per column how
/// many are left out. A column that leaves any out keeps its last lane for
/// the "+N more" line, so a bar in that lane is hidden if any of its days
/// overflows.
pub fn fit_lanes(bars: &[(BarSpan, usize)], columns: usize, cap: usize) -> (Vec<bool>, Vec<usize>) {
    let overflows: Vec<bool> = (0..columns)
        .map(|c| bars.iter().any(|(bar, lane)| bar.covers(c) && *lane >= cap))
        .collect();
    let shown: Vec<bool> = bars
        .iter()
        .map(|(bar, lane)| {
            *lane + 1 < cap
                || (*lane + 1 == cap && !(0..columns).any(|c| bar.covers(c) && overflows[c]))
        })
        .collect();
    let more = (0..columns)
        .map(|c| {
            bars.iter()
                .zip(&shown)
                .filter(|((bar, _), shown)| bar.covers(c) && !**shown)
                .count()
        })
        .collect();
    (shown, more)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    fn places(spans: &[Span]) -> Vec<(usize, usize, usize)> {
        pack_columns(spans)
            .into_iter()
            .map(|s| (s.column, s.columns, s.span))
            .collect()
    }

    #[test]
    fn separate_events_take_the_full_width() {
        assert_eq!(
            places(&[span(540, 600), span(600, 660), span(720, 780)]),
            [(0, 1, 1), (0, 1, 1), (0, 1, 1)]
        );
    }

    #[test]
    fn two_overlapping_events_split_the_width() {
        assert_eq!(
            places(&[span(540, 600), span(570, 630)]),
            [(0, 2, 1), (1, 2, 1)]
        );
    }

    #[test]
    fn the_longer_of_two_simultaneous_events_goes_left() {
        assert_eq!(
            places(&[span(540, 570), span(540, 660)]),
            [(1, 2, 1), (0, 2, 1)]
        );
    }

    #[test]
    fn a_chain_shares_one_cluster_and_reuses_columns() {
        // A overlaps B, B overlaps C, A and C do not: two columns, C back in column 0.
        assert_eq!(
            places(&[span(540, 600), span(570, 660), span(600, 690)]),
            [(0, 2, 1), (1, 2, 1), (0, 2, 1)]
        );
    }

    #[test]
    fn an_event_widens_over_columns_it_has_to_itself() {
        // Three at once make three columns; the later one only meets the long
        // first event, so it takes the two columns on the right.
        assert_eq!(
            places(&[
                span(540, 720),
                span(540, 600),
                span(540, 600),
                span(630, 700)
            ]),
            [(0, 3, 1), (1, 3, 1), (2, 3, 1), (1, 3, 2)]
        );
    }

    #[test]
    fn clusters_are_independent() {
        assert_eq!(
            places(&[
                span(540, 600),
                span(540, 600),
                span(540, 600),
                span(700, 760)
            ]),
            [(0, 3, 1), (1, 3, 1), (2, 3, 1), (0, 1, 1)]
        );
    }

    #[test]
    fn empty_day() {
        assert!(pack_columns(&[]).is_empty());
    }

    fn bar(first: usize, len: usize) -> BarSpan {
        BarSpan { first, len }
    }

    #[test]
    fn bars_stack_only_where_they_meet() {
        assert_eq!(
            pack_lanes(&[bar(0, 3), bar(2, 2), bar(3, 1), bar(4, 3)], 7),
            [0, 1, 0, 0]
        );
    }

    #[test]
    fn a_bar_past_the_row_is_clipped() {
        assert_eq!(pack_lanes(&[bar(5, 9), bar(6, 1)], 7), [0, 1]);
    }

    #[test]
    fn everything_fits_under_the_cap() {
        let bars = [(bar(0, 2), 0), (bar(1, 1), 1)];
        let (shown, more) = fit_lanes(&bars, 3, 2);
        assert_eq!(shown, [true, true]);
        assert_eq!(more, [0, 0, 0]);
    }

    #[test]
    fn an_overflowing_day_gives_up_its_last_lane() {
        // Column 1 has three bars but room for two lanes: it shows one bar and
        // "+2 more". The lane-1 bar also covers column 0, where it stays
        // hidden and counted.
        let bars = [(bar(0, 3), 0), (bar(0, 2), 1), (bar(1, 1), 2)];
        let (shown, more) = fit_lanes(&bars, 3, 2);
        assert_eq!(shown, [true, false, false]);
        assert_eq!(more, [1, 2, 0]);
    }

    #[test]
    fn no_room_hides_everything() {
        let bars = [(bar(0, 1), 0)];
        let (shown, more) = fit_lanes(&bars, 2, 0);
        assert_eq!(shown, [false]);
        assert_eq!(more, [1, 0]);
    }
}
