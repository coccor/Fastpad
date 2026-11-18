//! An SVG or FastDraw picture drawn by Direct2D on the graphics card, the way a browser draws one:
//! the parsed tree becomes Direct2D geometry once, and every paint replays it under the view's zoom
//! and pan. Zooming never renders pixels ahead of time, so there is no blurred picture waiting to
//! sharpen. A tree using what this does not draw (`drawable` says no: filters, masks, patterns,
//! embedded images, blend modes, nested clips) keeps the resvg raster path.

use resvg::tiny_skia::{PathSegment, Point, Transform};
use resvg::usvg;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END,
    D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_FILL_MODE, D2D1_FILL_MODE_ALTERNATE,
    D2D1_FILL_MODE_WINDING, D2D1_GRADIENT_STOP,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_CAP_STYLE, D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND,
    D2D1_CAP_STYLE_SQUARE, D2D1_COMBINE_MODE_UNION, D2D1_DASH_STYLE_CUSTOM, D2D1_DASH_STYLE_SOLID,
    D2D1_DEFAULT_FLATTENING_TOLERANCE, D2D1_EXTEND_MODE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_EXTEND_MODE_MIRROR, D2D1_EXTEND_MODE_WRAP, D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE,
    D2D1_LAYER_PARAMETERS, D2D1_LINE_JOIN, D2D1_LINE_JOIN_BEVEL, D2D1_LINE_JOIN_MITER_OR_BEVEL,
    D2D1_LINE_JOIN_ROUND, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_QUADRATIC_BEZIER_SEGMENT,
    D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_STROKE_STYLE_PROPERTIES, ID2D1Brush, ID2D1Factory,
    ID2D1Geometry, ID2D1Layer, ID2D1PathGeometry, ID2D1RenderTarget, ID2D1StrokeStyle,
};
use windows::core::{Interface, Result};
use windows_numerics::{Matrix3x2, Vector2};

/// Whether Direct2D can draw everything in `tree` as resvg would. Cheap: one walk of the tree.
pub(crate) fn drawable(tree: &usvg::Tree) -> bool {
    group_drawable(tree.root())
}

fn group_drawable(group: &usvg::Group) -> bool {
    group.mask().is_none()
        && group.filters().is_empty()
        && group.blend_mode() == usvg::BlendMode::Normal
        && group.clip_path().is_none_or(clip_drawable)
        && group.children().iter().all(node_drawable)
}

fn node_drawable(node: &usvg::Node) -> bool {
    match node {
        usvg::Node::Group(group) => group_drawable(group),
        usvg::Node::Path(path) => {
            path.fill().is_none_or(|fill| paint_drawable(fill.paint()))
                && path
                    .stroke()
                    .is_none_or(|stroke| paint_drawable(stroke.paint()))
        }
        usvg::Node::Text(text) => group_drawable(text.flattened()),
        usvg::Node::Image(_) => false,
    }
}

fn paint_drawable(paint: &usvg::Paint) -> bool {
    match paint {
        usvg::Paint::Color(_) | usvg::Paint::LinearGradient(_) => true,
        // Direct2D's radial gradient has no focal radius.
        usvg::Paint::RadialGradient(gradient) => gradient.fr().get() == 0.0,
        usvg::Paint::Pattern(_) => false,
    }
}

/// A clip path made only of shapes (and text), with no clip of its own anywhere inside.
fn clip_drawable(clip: &usvg::ClipPath) -> bool {
    fn shapes_only(group: &usvg::Group) -> bool {
        group.children().iter().all(|node| match node {
            usvg::Node::Path(_) => true,
            usvg::Node::Text(text) => shapes_only(text.flattened()),
            usvg::Node::Group(group) => group.clip_path().is_none() && shapes_only(group),
            usvg::Node::Image(_) => false,
        })
    }
    clip.clip_path().is_none() && shapes_only(clip.root())
}

/// What fills or strokes a shape, independent of any render target.
#[derive(Clone, Debug, PartialEq)]
enum Brush {
    Solid(D2D1_COLOR_F),
    Linear {
        start: Vector2,
        end: Vector2,
        stops: Vec<D2D1_GRADIENT_STOP>,
        extend: D2D1_EXTEND_MODE,
        transform: Matrix3x2,
    },
    Radial {
        center: Vector2,
        offset: Vector2,
        radius: f32,
        stops: Vec<D2D1_GRADIENT_STOP>,
        extend: D2D1_EXTEND_MODE,
        transform: Matrix3x2,
    },
}

/// One drawing step. Shapes are in their own space; `transform` takes it to the picture's.
enum Op {
    Fill {
        geometry: ID2D1Geometry,
        transform: Matrix3x2,
        brush: usize,
    },
    Stroke {
        geometry: ID2D1Geometry,
        transform: Matrix3x2,
        brush: usize,
        width: f32,
        style: Option<ID2D1StrokeStyle>,
    },
    /// A group's opacity and clip, in the picture's space, until the matching `Pop`.
    Push {
        opacity: f32,
        clip: Option<ID2D1Geometry>,
    },
    Pop,
}

/// A picture as Direct2D geometry, made once per parsed tree. The geometry belongs to the factory
/// and outlives render targets; the brushes belong to one target and are made again for a new one.
pub(crate) struct Scene {
    ops: Vec<Op>,
    brushes: Vec<Brush>,
    made: Option<Vec<ID2D1Brush>>,
    /// The picture's own size, which the view's zoom maps to its natural size.
    size: (f32, f32),
}

impl Scene {
    /// Turns `tree` into geometry. Call only for a tree `drawable` accepts.
    pub(crate) fn build(factory: &ID2D1Factory, tree: &usvg::Tree) -> Result<Self> {
        let mut builder = Builder {
            factory,
            ops: Vec::new(),
            brushes: Vec::new(),
        };
        builder.group_children(tree.root())?;
        let size = tree.size();
        Ok(Self {
            ops: builder.ops,
            brushes: builder.brushes,
            made: None,
            size: (size.width(), size.height()),
        })
    }

    /// Forgets the brushes of a render target that is gone.
    pub(crate) fn release_target(&mut self) {
        self.made = None;
    }

    /// Draws the picture with its top-left corner at `offset` and `size` across, in the target's
    /// pixels. Leaves the target's transform as it found it: identity.
    pub(crate) fn draw(
        &mut self,
        target: &ID2D1RenderTarget,
        offset: (f32, f32),
        size: (f32, f32),
    ) -> Result<()> {
        if self.made.is_none() {
            let made = self
                .brushes
                .iter()
                .map(|brush| make_brush(target, brush))
                .collect::<Result<Vec<_>>>()?;
            self.made = Some(made);
        }
        let made = self.made.as_ref().expect("made above");
        let view = Matrix3x2 {
            M11: size.0 / self.size.0,
            M12: 0.0,
            M21: 0.0,
            M22: size.1 / self.size.1,
            M31: offset.0,
            M32: offset.1,
        };
        for op in &self.ops {
            match op {
                Op::Fill {
                    geometry,
                    transform,
                    brush,
                } => unsafe {
                    target.SetTransform(&multiply(*transform, view));
                    target.FillGeometry(geometry, &made[*brush], None);
                },
                Op::Stroke {
                    geometry,
                    transform,
                    brush,
                    width,
                    style,
                } => unsafe {
                    target.SetTransform(&multiply(*transform, view));
                    target.DrawGeometry(geometry, &made[*brush], *width, style.as_ref());
                },
                Op::Push { opacity, clip } => unsafe {
                    target.SetTransform(&view);
                    let parameters = D2D1_LAYER_PARAMETERS {
                        contentBounds: D2D_RECT_F {
                            left: -f32::MAX,
                            top: -f32::MAX,
                            right: f32::MAX,
                            bottom: f32::MAX,
                        },
                        geometricMask: std::mem::ManuallyDrop::new(clip.clone()),
                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        maskTransform: Matrix3x2::identity(),
                        opacity: *opacity,
                        opacityBrush: std::mem::ManuallyDrop::new(None),
                        layerOptions: D2D1_LAYER_OPTIONS_NONE,
                    };
                    target.PushLayer(&parameters, None::<&ID2D1Layer>);
                    // The target took its own reference to the mask.
                    drop(std::mem::ManuallyDrop::into_inner(parameters.geometricMask));
                },
                Op::Pop => unsafe { target.PopLayer() },
            }
        }
        unsafe { target.SetTransform(&Matrix3x2::identity()) };
        Ok(())
    }

    #[cfg(test)]
    fn counts(&self) -> (usize, usize, usize) {
        let fills = self
            .ops
            .iter()
            .filter(|op| matches!(op, Op::Fill { .. }))
            .count();
        let strokes = self
            .ops
            .iter()
            .filter(|op| matches!(op, Op::Stroke { .. }))
            .count();
        let layers = self
            .ops
            .iter()
            .filter(|op| matches!(op, Op::Push { .. }))
            .count();
        (fills, strokes, layers)
    }
}

struct Builder<'a> {
    factory: &'a ID2D1Factory,
    ops: Vec<Op>,
    brushes: Vec<Brush>,
}

impl Builder<'_> {
    fn group_children(&mut self, group: &usvg::Group) -> Result<()> {
        for node in group.children() {
            match node {
                usvg::Node::Group(group) => self.group(group)?,
                usvg::Node::Path(path) => self.path(path)?,
                usvg::Node::Text(text) => self.group(text.flattened())?,
                usvg::Node::Image(_) => {}
            }
        }
        Ok(())
    }

    fn group(&mut self, group: &usvg::Group) -> Result<()> {
        let opacity = group.opacity().get();
        let clip = group
            .clip_path()
            .map(|clip| self.clip(clip, group.abs_transform()))
            .transpose()?;
        let layer = opacity < 1.0 || clip.is_some();
        if layer {
            self.ops.push(Op::Push { opacity, clip });
        }
        self.group_children(group)?;
        if layer {
            self.ops.push(Op::Pop);
        }
        Ok(())
    }

    fn path(&mut self, path: &usvg::Path) -> Result<()> {
        if !path.is_visible() {
            return Ok(());
        }
        let transform = matrix(path.abs_transform());
        let fill_rule = path
            .fill()
            .map_or(usvg::FillRule::NonZero, |fill| fill.rule());
        let geometry: ID2D1Geometry =
            path_geometry(self.factory, path.data(), fill_rule)?.cast()?;
        let fill = |builder: &mut Self| -> Result<()> {
            let Some(fill) = path.fill() else {
                return Ok(());
            };
            // As resvg: a shape with no area has nothing to fill.
            let bounds = path.data().bounds();
            if bounds.width() == 0.0 || bounds.height() == 0.0 {
                return Ok(());
            }
            if let Some(brush) = brush(fill.paint(), fill.opacity().get()) {
                builder.brushes.push(brush);
                builder.ops.push(Op::Fill {
                    geometry: geometry.clone(),
                    transform,
                    brush: builder.brushes.len() - 1,
                });
            }
            Ok(())
        };
        let stroke = |builder: &mut Self| -> Result<()> {
            let Some(stroke) = path.stroke() else {
                return Ok(());
            };
            if let Some(brush) = brush(stroke.paint(), stroke.opacity().get()) {
                builder.brushes.push(brush);
                let width = stroke.width().get();
                builder.ops.push(Op::Stroke {
                    geometry: geometry.clone(),
                    transform,
                    brush: builder.brushes.len() - 1,
                    width,
                    style: stroke_style(builder.factory, stroke, width)?,
                });
            }
            Ok(())
        };
        if path.paint_order() == usvg::PaintOrder::FillAndStroke {
            fill(self)?;
            stroke(self)
        } else {
            stroke(self)?;
            fill(self)
        }
    }

    /// The area `clip` lets through, in the picture's space: the union of its shapes.
    fn clip(&self, clip: &usvg::ClipPath, base: Transform) -> Result<ID2D1Geometry> {
        let mut shapes = Vec::new();
        self.clip_shapes(clip.root(), base.pre_concat(clip.transform()), &mut shapes)?;
        let mut shapes = shapes.into_iter();
        let Some(mut area) = shapes.next() else {
            // An empty clip path hides everything.
            return self.empty();
        };
        for shape in shapes {
            let union = unsafe { self.factory.CreatePathGeometry()? };
            let sink = unsafe { union.Open()? };
            unsafe {
                area.CombineWithGeometry(
                    &shape,
                    D2D1_COMBINE_MODE_UNION,
                    None,
                    D2D1_DEFAULT_FLATTENING_TOLERANCE / 64.0,
                    &sink,
                )?;
                sink.Close()?;
            }
            area = union.cast()?;
        }
        Ok(area)
    }

    fn clip_shapes(
        &self,
        group: &usvg::Group,
        transform: Transform,
        shapes: &mut Vec<ID2D1Geometry>,
    ) -> Result<()> {
        for node in group.children() {
            match node {
                usvg::Node::Path(path) if path.is_visible() => {
                    let rule = path
                        .fill()
                        .map_or(usvg::FillRule::NonZero, |fill| fill.rule());
                    let geometry = path_geometry(self.factory, path.data(), rule)?;
                    let placed = unsafe {
                        self.factory
                            .CreateTransformedGeometry(&geometry, &matrix(transform))?
                    };
                    shapes.push(placed.cast()?);
                }
                usvg::Node::Text(text) => self.clip_shapes(text.flattened(), transform, shapes)?,
                usvg::Node::Group(group) => {
                    self.clip_shapes(group, transform.pre_concat(group.transform()), shapes)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn empty(&self) -> Result<ID2D1Geometry> {
        let geometry = unsafe { self.factory.CreatePathGeometry()? };
        unsafe { geometry.Open()?.Close()? };
        geometry.cast()
    }
}

fn path_geometry(
    factory: &ID2D1Factory,
    data: &resvg::tiny_skia::Path,
    rule: usvg::FillRule,
) -> Result<ID2D1PathGeometry> {
    let geometry = unsafe { factory.CreatePathGeometry()? };
    let sink = unsafe { geometry.Open()? };
    let mode: D2D1_FILL_MODE = match rule {
        usvg::FillRule::NonZero => D2D1_FILL_MODE_WINDING,
        usvg::FillRule::EvenOdd => D2D1_FILL_MODE_ALTERNATE,
    };
    unsafe { sink.SetFillMode(mode) };
    let mut open = false;
    let mut start = Point::zero();
    let begin = |open: &mut bool, at: Point| {
        if !*open {
            unsafe { sink.BeginFigure(vector(at), D2D1_FIGURE_BEGIN_FILLED) };
            *open = true;
        }
    };
    let end = |open: &mut bool, how: D2D1_FIGURE_END| {
        if *open {
            unsafe { sink.EndFigure(how) };
            *open = false;
        }
    };
    for segment in data.segments() {
        match segment {
            PathSegment::MoveTo(point) => {
                end(&mut open, D2D1_FIGURE_END_OPEN);
                start = point;
                begin(&mut open, point);
            }
            // A segment after a close starts again where that figure began.
            PathSegment::LineTo(point) => {
                begin(&mut open, start);
                unsafe { sink.AddLine(vector(point)) };
            }
            PathSegment::QuadTo(control, point) => {
                begin(&mut open, start);
                unsafe {
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
                        point1: vector(control),
                        point2: vector(point),
                    })
                };
            }
            PathSegment::CubicTo(first, second, point) => {
                begin(&mut open, start);
                unsafe {
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: vector(first),
                        point2: vector(second),
                        point3: vector(point),
                    })
                };
            }
            PathSegment::Close => end(&mut open, D2D1_FIGURE_END_CLOSED),
        }
    }
    end(&mut open, D2D1_FIGURE_END_OPEN);
    unsafe { sink.Close()? };
    Ok(geometry)
}

fn stroke_style(
    factory: &ID2D1Factory,
    stroke: &usvg::Stroke,
    width: f32,
) -> Result<Option<ID2D1StrokeStyle>> {
    let cap: D2D1_CAP_STYLE = match stroke.linecap() {
        usvg::LineCap::Butt => D2D1_CAP_STYLE_FLAT,
        usvg::LineCap::Round => D2D1_CAP_STYLE_ROUND,
        usvg::LineCap::Square => D2D1_CAP_STYLE_SQUARE,
    };
    // SVG's miter falls back to a bevel past the limit, as MITER_OR_BEVEL does.
    let join: D2D1_LINE_JOIN = match stroke.linejoin() {
        usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => D2D1_LINE_JOIN_MITER_OR_BEVEL,
        usvg::LineJoin::Round => D2D1_LINE_JOIN_ROUND,
        usvg::LineJoin::Bevel => D2D1_LINE_JOIN_BEVEL,
    };
    // Direct2D measures dashes in stroke widths, SVG in user units.
    let dashes = stroke
        .dasharray()
        .filter(|_| width > 0.0)
        .map(|dashes| dashes.iter().map(|dash| dash / width).collect::<Vec<_>>());
    let properties = D2D1_STROKE_STYLE_PROPERTIES {
        startCap: cap,
        endCap: cap,
        dashCap: cap,
        lineJoin: join,
        miterLimit: stroke.miterlimit().get(),
        dashStyle: if dashes.is_some() {
            D2D1_DASH_STYLE_CUSTOM
        } else {
            D2D1_DASH_STYLE_SOLID
        },
        dashOffset: if dashes.is_some() {
            stroke.dashoffset() / width
        } else {
            0.0
        },
    };
    Ok(Some(unsafe {
        factory.CreateStrokeStyle(&properties, dashes.as_deref())?
    }))
}

/// The brush for `paint` at `opacity`, as resvg would paint it; `None` for nothing to paint.
fn brush(paint: &usvg::Paint, opacity: f32) -> Option<Brush> {
    let stops = |base: &usvg::BaseGradient| {
        base.stops()
            .iter()
            .map(|stop| D2D1_GRADIENT_STOP {
                position: stop.offset().get(),
                color: color(stop.color(), stop.opacity().get() * opacity),
            })
            .collect::<Vec<_>>()
    };
    let extend = |base: &usvg::BaseGradient| match base.spread_method() {
        usvg::SpreadMethod::Pad => D2D1_EXTEND_MODE_CLAMP,
        usvg::SpreadMethod::Reflect => D2D1_EXTEND_MODE_MIRROR,
        usvg::SpreadMethod::Repeat => D2D1_EXTEND_MODE_WRAP,
    };
    match paint {
        usvg::Paint::Color(value) => Some(Brush::Solid(color(*value, opacity))),
        usvg::Paint::LinearGradient(gradient) => {
            let stops = stops(gradient);
            (!stops.is_empty()).then(|| Brush::Linear {
                start: Vector2 {
                    X: gradient.x1(),
                    Y: gradient.y1(),
                },
                end: Vector2 {
                    X: gradient.x2(),
                    Y: gradient.y2(),
                },
                stops,
                extend: extend(gradient),
                transform: matrix(gradient.transform()),
            })
        }
        usvg::Paint::RadialGradient(gradient) => {
            let stops = stops(gradient);
            (!stops.is_empty()).then(|| Brush::Radial {
                center: Vector2 {
                    X: gradient.cx(),
                    Y: gradient.cy(),
                },
                offset: Vector2 {
                    X: gradient.fx() - gradient.cx(),
                    Y: gradient.fy() - gradient.cy(),
                },
                radius: gradient.r().get(),
                stops,
                extend: extend(gradient),
                transform: matrix(gradient.transform()),
            })
        }
        usvg::Paint::Pattern(_) => None,
    }
}

fn make_brush(target: &ID2D1RenderTarget, brush: &Brush) -> Result<ID2D1Brush> {
    unsafe {
        match brush {
            Brush::Solid(color) => target.CreateSolidColorBrush(color, None)?.cast(),
            Brush::Linear {
                start,
                end,
                stops,
                extend,
                transform,
            } => {
                let collection =
                    target.CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, *extend)?;
                let made = target.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: *start,
                        endPoint: *end,
                    },
                    None,
                    &collection,
                )?;
                made.SetTransform(transform);
                made.cast()
            }
            Brush::Radial {
                center,
                offset,
                radius,
                stops,
                extend,
                transform,
            } => {
                let collection =
                    target.CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, *extend)?;
                let made = target.CreateRadialGradientBrush(
                    &D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                        center: *center,
                        gradientOriginOffset: *offset,
                        radiusX: *radius,
                        radiusY: *radius,
                    },
                    None,
                    &collection,
                )?;
                made.SetTransform(transform);
                made.cast()
            }
        }
    }
}

fn color(color: usvg::Color, alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(color.red) / 255.0,
        g: f32::from(color.green) / 255.0,
        b: f32::from(color.blue) / 255.0,
        a: alpha.clamp(0.0, 1.0),
    }
}

fn vector(point: Point) -> Vector2 {
    Vector2 {
        X: point.x,
        Y: point.y,
    }
}

/// tiny-skia's column transform as Direct2D's row matrix.
fn matrix(transform: Transform) -> Matrix3x2 {
    Matrix3x2 {
        M11: transform.sx,
        M12: transform.ky,
        M21: transform.kx,
        M22: transform.sy,
        M31: transform.tx,
        M32: transform.ty,
    }
}

/// `first` then `second`.
fn multiply(first: Matrix3x2, second: Matrix3x2) -> Matrix3x2 {
    Matrix3x2 {
        M11: first.M11 * second.M11 + first.M12 * second.M21,
        M12: first.M11 * second.M12 + first.M12 * second.M22,
        M21: first.M21 * second.M11 + first.M22 * second.M21,
        M22: first.M21 * second.M12 + first.M22 * second.M22,
        M31: first.M31 * second.M11 + first.M32 * second.M21 + second.M31,
        M32: first.M31 * second.M12 + first.M32 * second.M22 + second.M32,
    }
}

#[cfg(test)]
mod tests {
    use super::{Scene, drawable, matrix, multiply};
    use crate::preview::svg::parse;
    use resvg::tiny_skia::Transform;
    use resvg::usvg;

    fn svg(body: &str) -> usvg::Tree {
        parse(&format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50">{body}</svg>"#
        ))
        .unwrap()
    }

    #[test]
    fn shapes_gradients_strokes_clips_and_opacity_are_drawn_on_the_gpu() {
        // Break caught: everyday SVGs (icons, logos, diagrams) left on the slow raster path.
        assert!(drawable(&svg(
            r##"<defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1"/></linearGradient>
            <clipPath id="c"><circle cx="50" cy="25" r="20"/></clipPath></defs>
            <g opacity="0.5" clip-path="url(#c)"><rect width="100" height="50" fill="url(#g)"/></g>
            <path d="M0 0 L100 50" stroke="#000" stroke-dasharray="4 2" stroke-linecap="round"/>"##
        )));
    }

    #[test]
    fn filters_masks_patterns_and_images_stay_on_resvg() {
        // Break caught: a blurred or masked SVG drawn by Direct2D without its effect.
        for body in [
            r#"<filter id="f"><feGaussianBlur stdDeviation="2"/></filter><rect width="10" height="10" filter="url(#f)"/>"#,
            r#"<mask id="m"><rect width="5" height="5" fill="white"/></mask><rect width="10" height="10" mask="url(#m)"/>"#,
            r#"<pattern id="p" width="4" height="4"><rect width="2" height="2"/></pattern><rect width="10" height="10" fill="url(#p)"/>"#,
            r#"<rect width="10" height="10" style="mix-blend-mode:multiply"/>"#,
        ] {
            assert!(!drawable(&svg(body)), "{body}");
        }
    }

    #[test]
    fn row_matrices_compose_in_drawing_order() {
        // Break caught: a shape's own transform applied after the zoom (shapes sliding as the
        // view zooms) rather than before it.
        let shape = matrix(Transform::from_translate(10.0, 0.0));
        let view = matrix(Transform::from_scale(2.0, 2.0));
        let both = multiply(shape, view);
        assert_eq!((both.M11, both.M31), (2.0, 20.0));
    }

    /// `tree` drawn by Direct2D at `size`, as premultiplied BGRA: offscreen, on a WIC bitmap.
    fn gpu_pixels(tree: &usvg::Tree, (width, height): (u32, u32)) -> Vec<u8> {
        use windows::Win32::Graphics::Direct2D::Common::{
            D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
        };
        use windows::Win32::Graphics::Direct2D::D2D1_RENDER_TARGET_PROPERTIES;
        use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
        use windows::Win32::Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
            WICBitmapCacheOnLoad,
        };
        use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
        let _com = crate::preview::images::ComScope::enter();
        let graphics = crate::preview::dwrite::Graphics::load().unwrap();
        unsafe {
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let bitmap = wic
                .CreateBitmap(
                    width,
                    height,
                    &GUID_WICPixelFormat32bppPBGRA,
                    WICBitmapCacheOnLoad,
                )
                .unwrap();
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            };
            let target = graphics
                .d2d
                .CreateWicBitmapRenderTarget(&bitmap, &properties)
                .unwrap();
            let mut scene = Scene::build(&graphics.d2d, tree).unwrap();
            target.BeginDraw();
            target.Clear(Some(&D2D1_COLOR_F::default()));
            scene
                .draw(&target, (0.0, 0.0), (width as f32, height as f32))
                .unwrap();
            target.EndDraw(None, None).unwrap();
            let mut pixels = vec![0; (width * height * 4) as usize];
            bitmap
                .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
                .unwrap();
            pixels
        }
    }

    /// Premultiplied BGRA over white, as a PNG in `FASTPAD_VECTOR_PNGS` when it is set.
    fn save(name: &str, pixels: &[u8], (width, height): (u32, u32)) {
        let Some(folder) = std::env::var_os("FASTPAD_VECTOR_PNGS") else {
            return;
        };
        let bgra = pixels
            .chunks(4)
            .flat_map(|pixel| {
                let over = |channel: u8| channel.saturating_add(255 - pixel[3]);
                [over(pixel[0]), over(pixel[1]), over(pixel[2]), 255]
            })
            .collect();
        let png = crate::platform::png::encode_png(&crate::platform::png::Pixels {
            width,
            height,
            bgra,
        })
        .unwrap();
        std::fs::write(
            std::path::Path::new(&folder).join(format!("{name}.png")),
            png,
        )
        .unwrap();
    }

    #[test]
    fn direct2d_draws_what_resvg_draws() {
        // Break caught: a transform, gradient, stroke style, clip or opacity translated wrong, so
        // the GPU picture differs from the one the raster path showed before.
        let samples = [
            (
                "gradients",
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120">
                <defs><linearGradient id="l" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#36c"/><stop offset="1" stop-color="#c63" stop-opacity="0.5"/></linearGradient>
                <radialGradient id="r" cx="0.5" cy="0.5" r="0.5" fx="0.3" fy="0.3"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#080"/></radialGradient>
                <linearGradient id="s" x2="0.25" spreadMethod="reflect"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
                <rect width="200" height="120" fill="url(#l)"/>
                <circle cx="60" cy="60" r="40" fill="url(#r)"/>
                <rect x="110" y="20" width="80" height="80" fill="url(#s)" transform="rotate(10 150 60)"/></svg>"##,
            ),
            (
                "strokes",
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60" width="200" height="120">
                <path d="M10 10 L50 50 L90 10" fill="none" stroke="#222" stroke-width="6" stroke-linejoin="round" stroke-linecap="round"/>
                <path d="M10 55 H90" stroke="#c00" stroke-width="3" stroke-dasharray="8 4" stroke-dashoffset="2"/>
                <path d="M20 20 Q50 0 80 20 T80 40" fill="none" stroke="#06c" stroke-width="2"/>
                <rect x="40" y="25" width="20" height="15" fill="#fc0" stroke="#000" stroke-width="2" paint-order="stroke"/></svg>"##,
            ),
            (
                "clips-and-opacity",
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120">
                <defs><clipPath id="c"><circle cx="100" cy="60" r="50"/><rect x="0" y="0" width="40" height="40"/></clipPath></defs>
                <g clip-path="url(#c)"><rect width="200" height="120" fill="#48c"/><path d="M0 120 L200 0" stroke="#fff" stroke-width="10"/></g>
                <g opacity="0.5"><rect x="120" y="30" width="60" height="60" fill="#c00"/><rect x="140" y="50" width="60" height="60" fill="#c00"/></g>
                <path fill-rule="evenodd" d="M10 70 h40 v40 h-40 z M20 80 h20 v20 h-20 z" fill="#080"/>
                <g transform="translate(100 100) skewX(20) scale(0.5)"><rect width="40" height="30" fill="#606"/></g></svg>"##,
            ),
            (
                "text",
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60">
                <text x="10" y="40" font-family="Segoe UI" font-size="28" fill="#123">Zoom 123</text></svg>"##,
            ),
        ];
        for (name, source) in samples {
            let tree = crate::preview::svg::parse(source).unwrap();
            assert!(drawable(&tree), "{name}");
            let size = crate::preview::svg::natural_size(&tree);
            let size = (size.0 * 2, size.1 * 2);
            let gpu = gpu_pixels(&tree, size);
            let cpu = crate::preview::svg::rasterize(&tree, size).unwrap();
            save(&format!("{name}-gpu"), &gpu, size);
            save(&format!("{name}-cpu"), &cpu, size);
            let differences = gpu
                .chunks(4)
                .zip(cpu.chunks(4))
                .map(|(a, b)| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap())
                .collect::<Vec<_>>();
            let mean =
                differences.iter().map(|&d| f64::from(d)).sum::<f64>() / differences.len() as f64;
            let far =
                differences.iter().filter(|&&d| d > 64).count() as f64 / differences.len() as f64;
            // Edges anti-alias a little differently; the picture must not.
            assert!(
                mean < 3.0 && far < 0.02,
                "{name}: mean {mean:.2}, far {far:.4}"
            );
        }
    }

    #[test]
    fn a_scene_has_one_step_per_fill_stroke_and_layer() {
        // Break caught: paint order or a group's layer lost in translation.
        let _graphics = crate::preview::dwrite::Graphics::load().unwrap();
        let tree = svg(
            r##"<g opacity="0.5"><rect width="10" height="10" fill="red" stroke="blue"/></g>
            <line x1="0" y1="0" x2="10" y2="0" stroke="black"/>"##,
        );
        let scene = Scene::build(&_graphics.d2d, &tree).unwrap();
        // The line has no area to fill.
        assert_eq!(scene.counts(), (1, 2, 1));
    }
}
