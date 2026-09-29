//! Where a dragged tab would land and what dropping it there does (split editors spec §6.1, §6.2).
//! Pure: no window. The window half is `tab_drag`.

use crate::document::DocumentId;
use crate::window::split_tree::{Direction, GroupId};
use crate::window::titlebar::{Point, Rect};

/// Which part of a group's content a point is in (spec §6.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Zone {
    Middle,
    /// The outer third on that side: a drop there splits the group that way.
    Edge(Direction),
}

/// What a drag's pointer is over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    /// Insertion point `index` (0 to the tab count) in `group`'s strip.
    Strip {
        group: GroupId,
        index: usize,
    },
    Content {
        group: GroupId,
        zone: Zone,
    },
}

/// The dragged view: its group, its document, its strip index and how many tabs its group has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Source {
    pub(crate) group: GroupId,
    pub(crate) document: DocumentId,
    pub(crate) index: usize,
    pub(crate) group_len: usize,
}

/// What a drop does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    /// Moves the tab at `from` to `to` in its own strip.
    Reorder {
        group: GroupId,
        from: usize,
        to: usize,
    },
    /// Puts the view into `group` at strip `index` (`None`: the end), moving it or, with `copy`,
    /// adding a second view. A group that already shows the document activates that view.
    Place {
        group: GroupId,
        index: Option<usize>,
        copy: bool,
    },
    /// Splits `group` towards `direction` and puts the view in the new group.
    Split {
        group: GroupId,
        direction: Direction,
        copy: bool,
    },
}

/// The zone of `content` that `point` is in: an edge's outer third, else the middle. In a corner
/// the nearer edge wins, in pixels; on a tie the top or bottom edge (plan amendment 1).
pub(crate) fn zone(content: Rect, point: Point) -> Zone {
    let width = (content.right - content.left).max(1);
    let height = (content.bottom - content.top).max(1);
    // Top and bottom first, so they win a tie.
    let edges = [
        (Direction::Up, point.y - content.top, height),
        (Direction::Down, content.bottom - 1 - point.y, height),
        (Direction::Left, point.x - content.left, width),
        (Direction::Right, content.right - 1 - point.x, width),
    ];
    let mut nearest: Option<(Direction, i32)> = None;
    for (direction, distance, extent) in edges {
        if i64::from(distance) * 3 >= i64::from(extent) {
            continue;
        }
        if nearest.is_none_or(|(_, best)| distance < best) {
            nearest = Some((direction, distance));
        }
    }
    nearest.map_or(Zone::Middle, |(direction, _)| Zone::Edge(direction))
}

/// The index a tab at `from` ends up at when dropped at insertion point `insertion` of its own
/// strip; `None` when that is where it already is.
pub(crate) fn reorder_destination(from: usize, insertion: usize) -> Option<usize> {
    let to = if insertion > from {
        insertion - 1
    } else {
        insertion
    };
    (to != from).then_some(to)
}

/// What dropping `source` on `target` does, `copy` being Ctrl; `None` for a drop that does
/// nothing (spec §6.1, §6.2, plan amendment 2).
pub(crate) fn decide(source: Source, target: Target, copy: bool) -> Option<Action> {
    match target {
        Target::Strip { group, index } if group == source.group => {
            reorder_destination(source.index, index).map(|to| Action::Reorder {
                group,
                from: source.index,
                to,
            })
        }
        Target::Strip { group, index } => Some(Action::Place {
            group,
            index: Some(index),
            copy,
        }),
        Target::Content {
            zone: Zone::Middle,
            group,
        } if group == source.group => None,
        Target::Content {
            zone: Zone::Middle,
            group,
        } => Some(Action::Place {
            group,
            index: None,
            copy,
        }),
        Target::Content {
            zone: Zone::Edge(_),
            group,
        } if group == source.group && source.group_len == 1 && !copy => None,
        Target::Content {
            zone: Zone::Edge(direction),
            group,
        } => Some(Action::Split {
            group,
            direction,
            copy,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Source, Target, Zone, decide, reorder_destination, zone};
    use crate::document::DocumentId;
    use crate::window::split_tree::{Direction, GroupId};
    use crate::window::titlebar::{Point, Rect};

    const CONTENT: Rect = Rect::new(0, 0, 900, 300);

    fn source(group: u32, index: usize, group_len: usize) -> Source {
        Source {
            group: GroupId(group),
            document: DocumentId(7),
            index,
            group_len,
        }
    }

    #[test]
    fn the_outer_thirds_are_edges_and_the_rest_is_the_middle() {
        // Break caught: halves instead of thirds, so every drop off-centre splits.
        assert_eq!(zone(CONTENT, Point::new(450, 150)), Zone::Middle);
        assert_eq!(
            zone(CONTENT, Point::new(10, 150)),
            Zone::Edge(Direction::Left)
        );
        assert_eq!(
            zone(CONTENT, Point::new(890, 150)),
            Zone::Edge(Direction::Right)
        );
        assert_eq!(
            zone(CONTENT, Point::new(450, 10)),
            Zone::Edge(Direction::Up)
        );
        assert_eq!(
            zone(CONTENT, Point::new(450, 290)),
            Zone::Edge(Direction::Down)
        );
        // Just inside and just outside the left third (300 px).
        assert_eq!(
            zone(CONTENT, Point::new(299, 150)),
            Zone::Edge(Direction::Left)
        );
        assert_eq!(zone(CONTENT, Point::new(300, 150)), Zone::Middle);
    }

    #[test]
    fn in_a_corner_the_nearer_edge_wins_and_a_tie_goes_to_top_or_bottom() {
        // Break caught: a corner always splitting sideways, or the tie flipping between runs.
        assert_eq!(
            zone(CONTENT, Point::new(5, 50)),
            Zone::Edge(Direction::Left)
        );
        assert_eq!(zone(CONTENT, Point::new(50, 5)), Zone::Edge(Direction::Up));
        assert_eq!(zone(CONTENT, Point::new(20, 20)), Zone::Edge(Direction::Up));
        assert_eq!(
            zone(CONTENT, Point::new(879, 279)),
            Zone::Edge(Direction::Down)
        );
    }

    #[test]
    fn a_zone_is_found_in_an_offset_rectangle() {
        // Break caught: zones measured from the window origin instead of the content's.
        let content = Rect::new(200, 100, 500, 400);
        assert_eq!(zone(content, Point::new(350, 250)), Zone::Middle);
        assert_eq!(
            zone(content, Point::new(210, 250)),
            Zone::Edge(Direction::Left)
        );
    }

    #[test]
    fn reordering_onto_its_own_place_is_nothing() {
        // Break caught: a click-sized drag in the strip reshuffling tabs.
        assert_eq!(reorder_destination(2, 2), None);
        assert_eq!(reorder_destination(2, 3), None);
        assert_eq!(reorder_destination(2, 0), Some(0));
        assert_eq!(reorder_destination(0, 3), Some(2));
    }

    #[test]
    fn its_own_strip_reorders_even_with_ctrl() {
        // Break caught: Ctrl in its own strip making a second view of a document in one group.
        let target = Target::Strip {
            group: GroupId(1),
            index: 0,
        };
        let expected = Some(Action::Reorder {
            group: GroupId(1),
            from: 2,
            to: 0,
        });
        assert_eq!(decide(source(1, 2, 3), target, false), expected);
        assert_eq!(decide(source(1, 2, 3), target, true), expected);
        let same = Target::Strip {
            group: GroupId(1),
            index: 3,
        };
        assert_eq!(decide(source(1, 2, 3), same, false), None);
    }

    #[test]
    fn another_strip_places_at_the_insertion_point_moving_or_copying() {
        // Break caught: Ctrl ignored, or the insertion point dropped for the end of the strip.
        let target = Target::Strip {
            group: GroupId(2),
            index: 1,
        };
        assert_eq!(
            decide(source(1, 0, 1), target, false),
            Some(Action::Place {
                group: GroupId(2),
                index: Some(1),
                copy: false
            })
        );
        assert_eq!(
            decide(source(1, 0, 1), target, true),
            Some(Action::Place {
                group: GroupId(2),
                index: Some(1),
                copy: true
            })
        );
    }

    #[test]
    fn a_content_middle_appends_to_another_group_and_is_nothing_on_its_own() {
        // Break caught: a drop back onto its own editor moving the tab to the end.
        let other = Target::Content {
            group: GroupId(2),
            zone: Zone::Middle,
        };
        assert_eq!(
            decide(source(1, 0, 2), other, false),
            Some(Action::Place {
                group: GroupId(2),
                index: None,
                copy: false
            })
        );
        let own = Target::Content {
            group: GroupId(1),
            zone: Zone::Middle,
        };
        assert_eq!(decide(source(1, 0, 2), own, false), None);
        assert_eq!(decide(source(1, 0, 2), own, true), None);
    }

    #[test]
    fn an_edge_splits_except_its_own_group_holding_only_that_tab_without_ctrl() {
        // Break caught: dragging a lone tab to its own edge closing its group and reopening it
        // beside the hole it left.
        let own_edge = Target::Content {
            group: GroupId(1),
            zone: Zone::Edge(Direction::Right),
        };
        assert_eq!(decide(source(1, 0, 1), own_edge, false), None);
        assert_eq!(
            decide(source(1, 0, 1), own_edge, true),
            Some(Action::Split {
                group: GroupId(1),
                direction: Direction::Right,
                copy: true
            })
        );
        assert_eq!(
            decide(source(1, 0, 2), own_edge, false),
            Some(Action::Split {
                group: GroupId(1),
                direction: Direction::Right,
                copy: false
            })
        );
        let other_edge = Target::Content {
            group: GroupId(2),
            zone: Zone::Edge(Direction::Down),
        };
        assert_eq!(
            decide(source(1, 0, 1), other_edge, false),
            Some(Action::Split {
                group: GroupId(2),
                direction: Direction::Down,
                copy: false
            })
        );
    }
}
