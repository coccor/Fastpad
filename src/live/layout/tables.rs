//! Table rows as one layout each (phase 3 plan, Decision 4 and Task 5): every pipe of a rendered
//! row is an inline object whose width moves the next cell to its column. Columns are sized to
//! their widest cell, measured by laying each row out once with zero-width spacers.

use crate::Result;
use crate::live::offsets::OffsetMap;
use crate::live::spans::Align;
use windows::Win32::Graphics::DirectWrite::IDWriteTextLayout;

/// Each cell's natural width in a row laid out with zero-width spacers: the distance from the
/// end of each object to the start of the next.
pub(super) fn measure_row(layout: &IDWriteTextLayout, offsets: &OffsetMap) -> Result<Vec<f32>> {
    let objects = offsets.objects();
    let mut widths = Vec::with_capacity(objects.len().saturating_sub(1));
    for pair in objects.windows(2) {
        let (start, _) = super::position_point(layout, pair[0] + 1)?;
        let (end, _) = super::position_point(layout, pair[1])?;
        widths.push((end - start).max(0.0));
    }
    Ok(widths)
}

/// Column widths: the widest cell of each column, plus `padding` on each side.
pub(super) fn columns<'a>(rows: impl IntoIterator<Item = &'a [f32]>, padding: f32) -> Vec<f32> {
    let mut columns: Vec<f32> = Vec::new();
    for row in rows {
        if columns.len() < row.len() {
            columns.resize(row.len(), 0.0);
        }
        for (column, width) in columns.iter_mut().zip(row) {
            *column = column.max(*width);
        }
    }
    columns
        .into_iter()
        .map(|width| width + 2.0 * padding)
        .collect()
}

/// The widths of a row's spacers (one more than its cells): each places the next cell in its
/// column after `padding`, with the column's slack before or after the cell as `aligns` says. The
/// last spacer also covers the columns the row has no cells for.
pub(super) fn spacers(cells: &[f32], columns: &[f32], aligns: &[Align], padding: f32) -> Vec<f32> {
    let mut spacers = vec![0.0; cells.len() + 1];
    for (index, (cell, column)) in cells.iter().zip(columns).enumerate() {
        let slack = (column - 2.0 * padding - cell).max(0.0);
        let before = match aligns.get(index).copied().unwrap_or(Align::None) {
            Align::Right => slack,
            Align::Center => slack / 2.0,
            Align::Left | Align::None => 0.0,
        };
        spacers[index] += padding + before;
        spacers[index + 1] += slack - before + padding;
    }
    let missing: f32 = columns.iter().skip(cells.len()).sum();
    if let Some(last) = spacers.last_mut() {
        *last += missing;
    }
    spacers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_the_widest_cells_plus_padding() {
        let rows: [&[f32]; 3] = [&[10.0, 5.0], &[3.0, 20.0, 7.0], &[]];
        assert_eq!(columns(rows, 2.0), vec![14.0, 24.0, 11.0]);
    }

    #[test]
    fn alignment_places_the_slack() {
        let columns = [24.0, 24.0, 24.0];
        let aligns = [Align::Left, Align::Right, Align::Center];
        // Cells 10, 10, 10 in columns of 20 inner width: slack 10 each. Between cells a spacer
        // holds the left cell's slack after it, both paddings and the right cell's slack before.
        assert_eq!(
            spacers(&[10.0, 10.0, 10.0], &columns, &aligns, 2.0),
            vec![2.0, 10.0 + 2.0 + 2.0 + 10.0, 2.0 + 2.0 + 5.0, 5.0 + 2.0]
        );
    }

    #[test]
    fn a_row_with_fewer_cells_leaves_the_rest_empty() {
        let spaced = spacers(&[4.0], &[10.0, 12.0], &[], 1.0);
        assert_eq!(spaced, vec![1.0, 4.0 + 1.0 + 12.0]);
        // The cell and its spacers fill both columns.
        assert_eq!(spaced.iter().sum::<f32>() + 4.0, 22.0);
    }
}
