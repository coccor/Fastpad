//! Where each element of a FastDraw diagram sits and the line that writes it, so a click on the
//! drawing finds its line and the caret's line outlines its elements (the VS Code preview's `hits`).
//! Coordinates are the scene's, which are the SVG's user units: its viewBox is `origin` and `size`.

use fastdraw_render::scene::Scene;

/// An element's outline: a box for shapes and text, the points of a connector.
#[derive(Clone, Debug, PartialEq)]
pub enum HitShape {
    Box { x: f64, y: f64, w: f64, h: f64 },
    Line(Vec<(f64, f64)>),
}

/// One element and the 1-based line that writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub line: u32,
    pub shape: HitShape,
}

/// A drawing's elements, back to front as the renderer stacks them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hits {
    /// The scene's top-left corner, the viewBox's first two numbers.
    pub origin: (f64, f64),
    /// The scene's size, the viewBox's last two numbers.
    pub size: (f64, f64),
    pub items: Vec<Hit>,
}

/// The scene's elements back to front: shapes, then connectors, then free text, as VS Code's
/// `hitsOf` lists them. Elements on no line are left out.
pub fn hits_of(scene: &Scene) -> Hits {
    let boxed = |line: u32, x: f64, y: f64, w: f64, h: f64| Hit {
        line,
        shape: HitShape::Box { x, y, w, h },
    };
    let shapes = scene
        .shapes
        .iter()
        .map(|s| boxed(s.line, s.x, s.y, s.w, s.h));
    let conns = scene.conns.iter().map(|c| Hit {
        line: c.line,
        shape: HitShape::Line(c.pts.iter().map(|p| (p.x, p.y)).collect()),
    });
    let texts = scene
        .texts
        .iter()
        .map(|t| boxed(t.line, t.x, t.y, t.w, t.h));
    Hits {
        origin: (scene.x, scene.y),
        size: (scene.w, scene.h),
        items: shapes
            .chain(conns)
            .chain(texts)
            .filter(|hit| hit.line > 0)
            .collect(),
    }
}

/// The line of the topmost element under `point` (scene units); a connector counts within
/// `tolerance` of its path.
pub fn hit_at(hits: &Hits, point: (f64, f64), tolerance: f64) -> Option<u32> {
    hits.items
        .iter()
        .rev()
        .find(|hit| contains(&hit.shape, point, tolerance))
        .map(|hit| hit.line)
}

fn contains(shape: &HitShape, (x, y): (f64, f64), tolerance: f64) -> bool {
    match shape {
        HitShape::Box { x: bx, y: by, w, h } => x >= *bx && x <= bx + w && y >= *by && y <= by + h,
        HitShape::Line(points) if points.len() == 1 => {
            (points[0].0 - x).hypot(points[0].1 - y) <= tolerance
        }
        HitShape::Line(points) => points
            .windows(2)
            .any(|pair| segment_distance(pair[0], pair[1], (x, y)) <= tolerance),
    }
}

/// The distance from `p` to the segment `a`–`b`, as `preview.js`'s `nearLine` measures it.
fn segment_distance(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx * dx + dy * dy;
    let t = if len > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a.0 + t * dx - p.0).hypot(a.1 + t * dy - p.1)
}

/// The outlines of every element written on `line`.
pub fn on_line(hits: &Hits, line: u32) -> impl Iterator<Item = &HitShape> {
    hits.items
        .iter()
        .filter(move |hit| hit.line == line)
        .map(|hit| &hit.shape)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastdraw_render::Source;

    fn boxed(line: u32, x: f64, y: f64, w: f64, h: f64) -> Hit {
        Hit {
            line,
            shape: HitShape::Box { x, y, w, h },
        }
    }

    fn hits(items: Vec<Hit>) -> Hits {
        Hits {
            items,
            ..Hits::default()
        }
    }

    fn scene(text: &str) -> Scene {
        fastdraw_render::drawn_scene(&Source {
            text,
            file: None,
            dark: false,
        })
    }

    #[test]
    fn hits_follow_the_scene_order_shapes_lines_texts() {
        let hits = hits_of(&scene(
            "a: rect \"A\" at 0,0\nb: rect \"B\" at 200,0\na -> b\n",
        ));
        let lines: Vec<u32> = hits.items.iter().map(|hit| hit.line).collect();
        assert_eq!(lines, [1, 2, 3]);
        assert!(matches!(hits.items[2].shape, HitShape::Line(_)));
    }

    #[test]
    fn the_later_of_two_overlapping_boxes_wins() {
        let hits = hits(vec![
            boxed(1, 0.0, 0.0, 100.0, 100.0),
            boxed(2, 50.0, 50.0, 100.0, 100.0),
        ]);
        assert_eq!(hit_at(&hits, (60.0, 60.0), 1.0), Some(2));
        assert_eq!(hit_at(&hits, (10.0, 10.0), 1.0), Some(1));
    }

    #[test]
    fn a_line_is_hit_within_the_tolerance_only() {
        let hits = hits(vec![Hit {
            line: 5,
            shape: HitShape::Line(vec![(0.0, 0.0), (100.0, 0.0)]),
        }]);
        assert_eq!(hit_at(&hits, (50.0, 5.0), 6.0), Some(5));
        assert_eq!(hit_at(&hits, (50.0, 7.0), 6.0), None);
        assert_eq!(hit_at(&hits, (106.0, 0.0), 6.0), Some(5));
        assert_eq!(hit_at(&hits, (107.0, 0.0), 6.0), None);
    }

    #[test]
    fn a_line_drawn_over_a_box_wins_over_the_box() {
        let hits = hits(vec![
            boxed(1, 0.0, 0.0, 100.0, 100.0),
            Hit {
                line: 2,
                shape: HitShape::Line(vec![(0.0, 50.0), (100.0, 50.0)]),
            },
        ]);
        assert_eq!(hit_at(&hits, (50.0, 52.0), 6.0), Some(2));
        assert_eq!(hit_at(&hits, (50.0, 20.0), 6.0), Some(1));
    }

    #[test]
    fn empty_space_hits_nothing() {
        let hits = hits_of(&scene("a: rect \"A\" at 0,0\n"));
        assert!(!hits.items.is_empty());
        assert_eq!(hit_at(&hits, (-500.0, -500.0), 6.0), None);
    }

    #[test]
    fn elements_on_line_zero_are_never_hits() {
        let hits = hits_of(&scene(
            "a: rect \"A\" at 0,0\nb: rect \"B\" at 200,0\na -> b \"go\"\n",
        ));
        assert!(!hits.items.is_empty());
        assert!(hits.items.iter().all(|hit| hit.line > 0));
    }

    #[test]
    fn on_line_lists_every_element_written_on_that_line() {
        let hits = hits(vec![
            boxed(4, 0.0, 0.0, 10.0, 10.0),
            boxed(3, 0.0, 0.0, 10.0, 10.0),
            boxed(4, 20.0, 0.0, 10.0, 10.0),
        ]);
        assert_eq!(on_line(&hits, 4).count(), 2);
        assert_eq!(on_line(&hits, 9).count(), 0);
    }

    #[test]
    fn the_hits_carry_the_scene_frame() {
        let sc = scene("a: rect \"A\" at 0,0\n");
        let hits = hits_of(&sc);
        assert_eq!(hits.origin, (sc.x, sc.y));
        assert_eq!(hits.size, (sc.w, sc.h));
    }
}
