//! Logical slots refer to one source cell, including every covered span slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HtmlCellSpan {
    pub columns: usize,
    /// Zero extends to the end of this row group.
    pub rows: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HtmlGridCell {
    pub source_row: usize,
    pub source_cell: usize,
    pub column: usize,
    pub columns: usize,
    pub rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlTableGrid {
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<HtmlGridCell>,
    pub slots: Vec<Option<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlGridError {
    InvalidSpan,
    Overlap,
    ResourceLimit,
}

impl HtmlTableGrid {
    /// Each outer item is a row group; cells and owners retain source order.
    pub fn build(groups: &[Vec<Vec<HtmlCellSpan>>]) -> Result<Self, HtmlGridError> {
        const MAX_SLOTS: usize = 1_048_576;
        let rows = groups
            .iter()
            .try_fold(0usize, |n, group| n.checked_add(group.len()))
            .filter(|n| *n <= MAX_SLOTS)
            .ok_or(HtmlGridError::ResourceLimit)?;
        let mut slots: Vec<Vec<Option<usize>>> = vec![Vec::new(); rows];
        let mut cells = Vec::new();
        let mut row_base = 0;
        let mut columns = 0;
        for group in groups {
            for (local_row, row) in group.iter().enumerate() {
                let y = row_base + local_row;
                let mut x = 0;
                for (source_cell, span) in row.iter().enumerate() {
                    if span.columns == 0 {
                        return Err(HtmlGridError::InvalidSpan);
                    }
                    while slots[y].get(x).is_some_and(Option::is_some) {
                        x += 1;
                    }
                    let end = x
                        .checked_add(span.columns)
                        .ok_or(HtmlGridError::ResourceLimit)?;
                    if end.checked_mul(rows).is_none_or(|size| size > MAX_SLOTS) {
                        return Err(HtmlGridError::ResourceLimit);
                    }
                    let remaining = group.len() - local_row;
                    let height = if span.rows == 0 {
                        remaining
                    } else {
                        span.rows.min(remaining)
                    };
                    let owner = cells.len();
                    for line in &mut slots[y..y + height] {
                        line.resize(line.len().max(end), None);
                        for slot in &mut line[x..end] {
                            if slot.is_some() {
                                return Err(HtmlGridError::Overlap);
                            }
                            *slot = Some(owner);
                        }
                    }
                    cells.push(HtmlGridCell {
                        source_row: y,
                        source_cell,
                        column: x,
                        columns: span.columns,
                        rows: height,
                    });
                    columns = columns.max(end);
                    x = end;
                }
            }
            row_base += group.len();
        }
        let slots = slots
            .into_iter()
            .flat_map(|mut row| {
                row.resize(columns, None);
                row
            })
            .collect();
        Ok(Self {
            columns,
            rows,
            cells,
            slots,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn span(columns: usize, rows: usize) -> HtmlCellSpan {
        HtmlCellSpan { columns, rows }
    }
    #[test]
    fn mixed_spans_have_one_owner_per_source_cell() {
        let grid = HtmlTableGrid::build(&[vec![
            vec![span(2, 2), span(1, 1)],
            vec![span(1, 1)],
            vec![span(1, 1)],
        ]])
        .expect("grid");
        assert_eq!((grid.columns, grid.rows), (3, 3));
        assert_eq!(
            grid.slots,
            vec![
                Some(0),
                Some(0),
                Some(1),
                Some(0),
                Some(0),
                Some(2),
                Some(3),
                None,
                None
            ]
        );
        assert_eq!(grid.cells.len(), 4);
        assert_eq!(
            (
                grid.cells[2].source_row,
                grid.cells[2].source_cell,
                grid.cells[2].column
            ),
            (1, 0, 2)
        );
    }
    #[test]
    fn row_spans_stop_at_group_boundary_including_zero() {
        for height in [0, 999] {
            let grid = HtmlTableGrid::build(&[
                vec![vec![span(1, height)], vec![]],
                vec![vec![span(1, 1)]],
            ])
            .expect("groups");
            assert_eq!(grid.slots, vec![Some(0), Some(0), Some(1)]);
            assert_eq!(grid.cells[0].rows, 2);
        }
    }
    #[test]
    fn collision_and_excessive_dimensions_are_rejected() {
        assert_eq!(
            HtmlTableGrid::build(&[vec![vec![span(1, 1), span(1, 2)], vec![span(2, 1)]]]),
            Err(HtmlGridError::Overlap)
        );
        assert_eq!(
            HtmlTableGrid::build(&[vec![vec![span(usize::MAX, 1)]]]),
            Err(HtmlGridError::ResourceLimit)
        );
        assert_eq!(
            HtmlTableGrid::build(&[vec![vec![span(0, 1)]]]),
            Err(HtmlGridError::InvalidSpan)
        );
    }
}
