//! An SVG or FastDraw picture drawn by Direct2D on the graphics card, the way a browser draws one:
//! the parsed tree becomes Direct2D geometry once, and every paint replays it under the view's zoom
//! and pan. Zooming never renders pixels ahead of time, so there is no blurred picture waiting to
//! sharpen. A pattern's tile is drawn again, sharp, whenever the zoom changes its size on screen.
//! Embedded SVG images (FastDraw's icons) are drawn as part of the tree. A group with a filter
//! (drop shadows, blurs) or a mask is drawn offscreen at the view's zoom each paint, limited to
//! what shows, and finished by Direct2D effects (`effects`). A tree using what this does not draw
//! (`drawable` says no: other filter primitives, embedded raster images, blend modes, nested
//! clips) keeps the resvg raster path.

use resvg::tiny_skia::{PathSegment, Point, Transform};
use resvg::usvg;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COMPOSITE_MODE_SOURCE_OVER;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D_SIZE_U, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F,
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
    D2D1_FILL_MODE, D2D1_FILL_MODE_ALTERNATE, D2D1_FILL_MODE_WINDING, D2D1_GRADIENT_STOP,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_BRUSH_PROPERTIES,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BRUSH_PROPERTIES, D2D1_CAP_STYLE,
    D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND, D2D1_CAP_STYLE_SQUARE, D2D1_COMBINE_MODE_UNION,
    D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE, D2D1_DASH_STYLE_CUSTOM, D2D1_DASH_STYLE_SOLID,
    D2D1_DEFAULT_FLATTENING_TOLERANCE, D2D1_EXTEND_MODE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_EXTEND_MODE_MIRROR, D2D1_EXTEND_MODE_WRAP, D2D1_GAMMA_2_2,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS,
    D2D1_LINE_JOIN, D2D1_LINE_JOIN_BEVEL, D2D1_LINE_JOIN_MITER_OR_BEVEL, D2D1_LINE_JOIN_ROUND,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_QUADRATIC_BEZIER_SEGMENT,
    D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_STROKE_STYLE_PROPERTIES, ID2D1Bitmap, ID2D1Brush,
    ID2D1DeviceContext, ID2D1Factory, ID2D1Geometry, ID2D1Image, ID2D1Layer, ID2D1PathGeometry,
    ID2D1RenderTarget, ID2D1StrokeStyle,
};
use windows::core::{Interface, Result};
use windows_numerics::{Matrix3x2, Vector2};

/// Whether Direct2D can draw everything in `tree` as resvg would. Cheap: one walk of the tree.
pub(crate) fn drawable(tree: &usvg::Tree) -> bool {
    group_drawable(tree.root())
}

fn group_drawable(group: &usvg::Group) -> bool {
    group.mask().is_none_or(mask_drawable)
        && group
            .filters()
            .iter()
            .all(|filter| super::effects::filter_drawable(filter))
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
        // An embedded SVG (FastDraw's icons) is a tree like any other; a raster image is not.
        usvg::Node::Image(image) => match image.kind() {
            usvg::ImageKind::SVG(tree) => !image.is_visible() || group_drawable(tree.root()),
            _ => false,
        },
    }
}

fn paint_drawable(paint: &usvg::Paint) -> bool {
    match paint {
        usvg::Paint::Color(_) | usvg::Paint::LinearGradient(_) => true,
        // Direct2D's radial gradient has no focal radius.
        usvg::Paint::RadialGradient(gradient) => gradient.fr().get() == 0.0,
        usvg::Paint::Pattern(pattern) => group_drawable(pattern.root()),
    }
}

fn mask_drawable(mask: &usvg::Mask) -> bool {
    group_drawable(mask.root()) && mask.mask().is_none_or(mask_drawable)
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
    /// `Scene::patterns[index]`, tiled.
    Pattern {
        index: usize,
        opacity: f32,
    },
}

/// A pattern: its content as a scene of its own, drawn into a tile at the zoom it shows at.
struct PatternSpec {
    scene: Scene,
    /// The tile's rectangle in the pattern's space.
    rect: (f32, f32, f32, f32),
    transform: Transform,
}

/// The largest side a pattern tile is drawn at; past it the tile is drawn enlarged.
const MAX_TILE: f32 = 4096.0;

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
    /// A group with filters or a mask: drawn offscreen and finished by effects.
    Effect(Box<EffectOp>),
}

/// A group drawn offscreen at each paint, as resvg draws an isolated group: its content, then its
/// filters, its clip, its mask, and its opacity.
struct EffectOp {
    ops: Vec<Op>,
    /// The group's space to the picture's.
    transform: Transform,
    /// What the group can cover with its filters, in its space.
    layer: Option<resvg::tiny_skia::NonZeroRect>,
    filters: Vec<std::sync::Arc<usvg::filter::Filter>>,
    mask: Option<MaskOps>,
    clip: Option<ID2D1Geometry>,
    opacity: f32,
}

/// A mask's content, drawn into its own layer, clipped to its rectangle (in the masked group's
/// space). As resvg: a mask of the mask applies to the masked content.
struct MaskOps {
    ops: Vec<Op>,
    rect: resvg::tiny_skia::NonZeroRect,
    kind: usvg::MaskType,
    empty: bool,
    mask: Option<Box<MaskOps>>,
}

/// A picture as Direct2D geometry, made once per parsed tree. The geometry belongs to the factory
/// and outlives render targets; the brushes belong to one target and are made again for a new one.
pub(crate) struct Scene {
    ops: Vec<Op>,
    brushes: Vec<Brush>,
    /// `brushes` made on the current target; `None` for a pattern, whose brush is in `tiles`.
    made: Option<Vec<Option<ID2D1Brush>>>,
    patterns: Vec<PatternSpec>,
    /// Each pattern brush's tile brush, by brush, with the tile's pixel size it was drawn at.
    tiles: std::collections::HashMap<usize, ((u32, u32), ID2D1Brush)>,
    /// The picture's own size, which the view's zoom maps to its natural size.
    size: (f32, f32),
}

impl Scene {
    /// Turns `tree` into geometry. Call only for a tree `drawable` accepts.
    pub(crate) fn build(factory: &ID2D1Factory, tree: &usvg::Tree) -> Result<Self> {
        let size = tree.size();
        Self::of_group(factory, tree.root(), (size.width(), size.height()))
    }

    fn of_group(factory: &ID2D1Factory, root: &usvg::Group, size: (f32, f32)) -> Result<Self> {
        let mut builder = Builder {
            factory,
            ops: Vec::new(),
            brushes: Vec::new(),
            patterns: Vec::new(),
        };
        // As resvg: the root's children, each group adding its own transform.
        builder.group_children(root, Transform::identity())?;
        Ok(Self {
            ops: builder.ops,
            brushes: builder.brushes,
            made: None,
            patterns: builder.patterns,
            tiles: std::collections::HashMap::new(),
            size,
        })
    }

    /// Forgets the brushes of a render target that is gone.
    pub(crate) fn release_target(&mut self) {
        self.made = None;
        self.tiles.clear();
        for pattern in &mut self.patterns {
            pattern.scene.release_target();
        }
    }

    /// Draws the picture with its top-left corner at `offset` and `size` across, in the target's
    /// pixels. Leaves the target's transform as it found it: identity.
    pub(crate) fn draw(
        &mut self,
        target: &ID2D1RenderTarget,
        offset: (f32, f32),
        size: (f32, f32),
    ) -> Result<()> {
        let view = Matrix3x2 {
            M11: size.0 / self.size.0,
            M12: 0.0,
            M21: 0.0,
            M22: size.1 / self.size.1,
            M31: offset.0,
            M32: offset.1,
        };
        self.draw_under(target, view)?;
        unsafe { target.SetTransform(&Matrix3x2::identity()) };
        Ok(())
    }

    /// Draws the picture with `view` taking its space to the target's.
    fn draw_under(&mut self, target: &ID2D1RenderTarget, view: Matrix3x2) -> Result<()> {
        if self.made.is_none() {
            let made = self
                .brushes
                .iter()
                .map(|brush| make_brush(target, brush))
                .collect::<Result<Vec<_>>>()?;
            self.made = Some(made);
        }
        let Self {
            ops,
            brushes,
            made,
            patterns,
            tiles,
            ..
        } = self;
        let mut painter = Painter {
            made: made.as_ref().expect("made above"),
            brushes,
            patterns,
            tiles,
        };
        painter.ops(target, ops, view)
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

/// Draws a scene's steps, making pattern tiles as the zoom needs them.
struct Painter<'a> {
    made: &'a [Option<ID2D1Brush>],
    brushes: &'a [Brush],
    patterns: &'a mut [PatternSpec],
    tiles: &'a mut std::collections::HashMap<usize, ((u32, u32), ID2D1Brush)>,
}

impl Painter<'_> {
    /// The brush for step `brush` drawn under `device`: a pattern's tile at that zoom.
    fn resolve(
        &mut self,
        target: &ID2D1RenderTarget,
        brush: usize,
        device: Matrix3x2,
    ) -> Result<Option<ID2D1Brush>> {
        if let Some(made) = &self.made[brush] {
            return Ok(Some(made.clone()));
        }
        let Brush::Pattern { index, opacity } = self.brushes[brush] else {
            return Ok(None);
        };
        let spec = &mut self.patterns[index];
        let Some((pixels, scale)) = tile_size(spec, device) else {
            return Ok(None);
        };
        if let Some((drawn, tile)) = self.tiles.get(&brush)
            && *drawn == pixels
        {
            return Ok(Some(tile.clone()));
        }
        let tile = pattern_tile(target, spec, pixels, scale, opacity)?;
        self.tiles.insert(brush, (pixels, tile.clone()));
        Ok(Some(tile))
    }

    fn ops(&mut self, target: &ID2D1RenderTarget, ops: &[Op], view: Matrix3x2) -> Result<()> {
        for op in ops {
            match op {
                Op::Fill {
                    geometry,
                    transform,
                    brush,
                } => {
                    let device = multiply(*transform, view);
                    if let Some(brush) = self.resolve(target, *brush, device)? {
                        unsafe {
                            target.SetTransform(&device);
                            target.FillGeometry(geometry, &brush, None);
                        }
                    }
                }
                Op::Stroke {
                    geometry,
                    transform,
                    brush,
                    width,
                    style,
                } => {
                    let device = multiply(*transform, view);
                    if let Some(brush) = self.resolve(target, *brush, device)? {
                        unsafe {
                            target.SetTransform(&device);
                            target.DrawGeometry(geometry, &brush, *width, style.as_ref());
                        }
                    }
                }
                Op::Push { opacity, clip } => unsafe {
                    target.SetTransform(&view);
                    push_layer(target, *opacity, clip.clone());
                },
                Op::Pop => unsafe { target.PopLayer() },
                Op::Effect(effect) => self.effect(target, effect, view)?,
            }
        }
        Ok(())
    }

    /// Draws `op`'s group into a layer covering what shows of it (and what its filters reach in
    /// from outside the view), applies its filters and mask, and draws the result under its clip
    /// and opacity.
    fn effect(&mut self, target: &ID2D1RenderTarget, op: &EffectOp, view: Matrix3x2) -> Result<()> {
        let device = multiply(matrix(op.transform), view);
        let ts = transform(device);
        let Some(bounds) = op.layer.and_then(|layer| layer.transform(ts)) else {
            return Ok(());
        };
        let view_size = unsafe { target.GetSize() };
        let reach = super::effects::reach(&op.filters, ts.get_scale()).min(4096.0);
        // As resvg: an unfiltered layer is widened 2 pixels for its anti-aliased edges.
        let pad = if op.filters.is_empty() { 2.0 } else { 0.0 };
        let left = (bounds.left() - pad).floor().max(-reach.ceil());
        let top = (bounds.top() - pad).floor().max(-reach.ceil());
        let right = (bounds.right() + pad)
            .ceil()
            .min((view_size.width + reach).ceil());
        let bottom = (bounds.bottom() + pad)
            .ceil()
            .min((view_size.height + reach).ceil());
        if right <= left || bottom <= top {
            return Ok(());
        }
        let size = ((right - left) as u32, (bottom - top) as u32);
        let layer_view = Matrix3x2 {
            M31: view.M31 - left,
            M32: view.M32 - top,
            ..view
        };
        let layer_ts = transform(multiply(matrix(op.transform), layer_view));
        let context: ID2D1DeviceContext = target.cast()?;
        let mut image: ID2D1Image = self
            .offscreen(target, &op.ops, layer_view, size, None)?
            .cast()?;
        for filter in &op.filters {
            match super::effects::apply_filter(&context, filter, layer_ts, &image, size)? {
                Some(filtered) => image = filtered,
                None => return Ok(()),
            }
        }
        if let Some(mask) = &op.mask {
            match self.masked(target, &context, image, mask, layer_view, layer_ts, size)? {
                Some(masked) => image = masked,
                None => return Ok(()),
            }
        }
        let layer = op.clip.is_some() || op.opacity < 1.0;
        unsafe {
            if layer {
                target.SetTransform(&view);
                push_layer(target, op.opacity, op.clip.clone());
            }
            target.SetTransform(&Matrix3x2::identity());
            context.DrawImage(
                &image,
                Some(&Vector2 { X: left, Y: top }),
                None,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
            if layer {
                target.PopLayer();
            }
        }
        Ok(())
    }

    /// `content` under `mask`; `None` when the mask hides it all.
    #[allow(clippy::too_many_arguments)]
    fn masked(
        &mut self,
        target: &ID2D1RenderTarget,
        context: &ID2D1DeviceContext,
        content: ID2D1Image,
        mask: &MaskOps,
        view: Matrix3x2,
        ts: Transform,
        size: (u32, u32),
    ) -> Result<Option<ID2D1Image>> {
        if mask.empty {
            return Ok(None);
        }
        let factory = unsafe { target.GetFactory()? };
        let rect = unsafe {
            factory.CreateRectangleGeometry(&D2D_RECT_F {
                left: mask.rect.left(),
                top: mask.rect.top(),
                right: mask.rect.right(),
                bottom: mask.rect.bottom(),
            })?
        };
        let shape: ID2D1Image = self
            .offscreen(
                target,
                &mask.ops,
                view,
                size,
                Some((rect.cast()?, matrix(ts))),
            )?
            .cast()?;
        let mut content = content;
        if let Some(inner) = &mask.mask {
            match self.masked(target, context, content, inner, view, ts, size)? {
                Some(masked) => content = masked,
                None => return Ok(None),
            }
        }
        super::effects::apply_mask(context, &content, &shape, mask.kind).map(Some)
    }

    /// `ops` drawn under `view` into a new `size` bitmap of `target`'s device, inside `clip`
    /// (a geometry and the transform it is drawn under) when one is given.
    fn offscreen(
        &mut self,
        target: &ID2D1RenderTarget,
        ops: &[Op],
        view: Matrix3x2,
        size: (u32, u32),
        clip: Option<(ID2D1Geometry, Matrix3x2)>,
    ) -> Result<ID2D1Bitmap> {
        unsafe {
            let layer = target.CreateCompatibleRenderTarget(
                Some(&D2D_SIZE_F {
                    width: size.0 as f32,
                    height: size.1 as f32,
                }),
                Some(&D2D_SIZE_U {
                    width: size.0,
                    height: size.1,
                }),
                None,
                D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE,
            )?;
            let surface: ID2D1RenderTarget = layer.cast()?;
            surface.BeginDraw();
            surface.Clear(Some(&D2D1_COLOR_F::default()));
            if let Some((geometry, transform)) = &clip {
                surface.SetTransform(transform);
                push_layer(&surface, 1.0, Some(geometry.clone()));
            }
            let drawn = self.ops(&surface, ops, view);
            if clip.is_some() {
                surface.PopLayer();
            }
            surface.SetTransform(&Matrix3x2::identity());
            surface.EndDraw(None, None)?;
            drawn?;
            layer.GetBitmap()
        }
    }
}

/// Pushes a layer of `opacity` clipped to `clip` under the target's current transform.
unsafe fn push_layer(target: &ID2D1RenderTarget, opacity: f32, clip: Option<ID2D1Geometry>) {
    let parameters = D2D1_LAYER_PARAMETERS {
        contentBounds: D2D_RECT_F {
            left: -f32::MAX,
            top: -f32::MAX,
            right: f32::MAX,
            bottom: f32::MAX,
        },
        geometricMask: std::mem::ManuallyDrop::new(clip),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2::identity(),
        opacity,
        opacityBrush: std::mem::ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS_NONE,
    };
    unsafe { target.PushLayer(&parameters, None::<&ID2D1Layer>) };
    // The target took its own reference to the mask.
    drop(std::mem::ManuallyDrop::into_inner(parameters.geometricMask));
}

/// Direct2D's row matrix as tiny-skia's transform.
fn transform(matrix: Matrix3x2) -> Transform {
    Transform::from_row(
        matrix.M11, matrix.M12, matrix.M21, matrix.M22, matrix.M31, matrix.M32,
    )
}

struct Builder<'a> {
    factory: &'a ID2D1Factory,
    ops: Vec<Op>,
    brushes: Vec<Brush>,
    patterns: Vec<PatternSpec>,
}

impl Builder<'_> {
    fn group_children(&mut self, group: &usvg::Group, transform: Transform) -> Result<()> {
        for node in group.children() {
            match node {
                usvg::Node::Group(group) => self.group(group, transform)?,
                usvg::Node::Path(path) => self.path(path, transform)?,
                usvg::Node::Text(text) => self.group(text.flattened(), transform)?,
                // As resvg: the embedded tree drawn where the image's group placed it.
                usvg::Node::Image(image) => {
                    if let usvg::ImageKind::SVG(tree) = image.kind()
                        && image.is_visible()
                    {
                        self.group_children(tree.root(), transform)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn group(&mut self, group: &usvg::Group, parent: Transform) -> Result<()> {
        let transform = parent.pre_concat(group.transform());
        let opacity = group.opacity().get();
        let clip = group
            .clip_path()
            .map(|clip| self.clip(clip, transform))
            .transpose()?;
        if !group.filters().is_empty() || group.mask().is_some() {
            let ops = self.nested(|builder| builder.group_children(group, transform))?;
            let mask = group
                .mask()
                .map(|mask| self.mask(mask, transform))
                .transpose()?;
            self.ops.push(Op::Effect(Box::new(EffectOp {
                ops,
                transform,
                layer: Some(group.layer_bounding_box()),
                filters: group.filters().to_vec(),
                mask,
                clip,
                opacity,
            })));
            return Ok(());
        }
        let layer = opacity < 1.0 || clip.is_some();
        if layer {
            self.ops.push(Op::Push { opacity, clip });
        }
        self.group_children(group, transform)?;
        if layer {
            self.ops.push(Op::Pop);
        }
        Ok(())
    }

    /// The steps `build` adds, kept apart from the ones before them.
    fn nested(&mut self, build: impl FnOnce(&mut Self) -> Result<()>) -> Result<Vec<Op>> {
        let before = std::mem::take(&mut self.ops);
        let built = build(self);
        let ops = std::mem::replace(&mut self.ops, before);
        built.map(|()| ops)
    }

    /// `mask`'s content, placed in the masked group's space `transform`.
    fn mask(&mut self, mask: &usvg::Mask, transform: Transform) -> Result<MaskOps> {
        let ops = self.nested(|builder| builder.group_children(mask.root(), transform))?;
        let inner = mask
            .mask()
            .map(|inner| self.mask(inner, transform).map(Box::new))
            .transpose()?;
        Ok(MaskOps {
            ops,
            rect: mask.rect(),
            kind: mask.kind(),
            empty: mask.root().children().is_empty(),
            mask: inner,
        })
    }

    /// The brush for `paint`; a pattern's content becomes a scene of its own.
    fn paint(&mut self, paint: &usvg::Paint, opacity: f32) -> Result<Option<Brush>> {
        let usvg::Paint::Pattern(pattern) = paint else {
            return Ok(brush(paint, opacity));
        };
        let rect = pattern.rect();
        let scene = Scene::of_group(self.factory, pattern.root(), (rect.width(), rect.height()))?;
        self.patterns.push(PatternSpec {
            scene,
            rect: (rect.x(), rect.y(), rect.width(), rect.height()),
            transform: pattern.transform(),
        });
        Ok(Some(Brush::Pattern {
            index: self.patterns.len() - 1,
            opacity,
        }))
    }

    fn path(&mut self, path: &usvg::Path, transform: Transform) -> Result<()> {
        if !path.is_visible() {
            return Ok(());
        }
        let transform = matrix(transform);
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
            if let Some(brush) = builder.paint(fill.paint(), fill.opacity().get())? {
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
            if let Some(brush) = builder.paint(stroke.paint(), stroke.opacity().get())? {
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

/// `brush` made on `target`; `None` for a pattern, made per zoom by `pattern_tile`.
fn make_brush(target: &ID2D1RenderTarget, brush: &Brush) -> Result<Option<ID2D1Brush>> {
    unsafe {
        match brush {
            Brush::Pattern { .. } => Ok(None),
            Brush::Solid(color) => Ok(Some(target.CreateSolidColorBrush(color, None)?.cast()?)),
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
                Ok(Some(made.cast()?))
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
                Ok(Some(made.cast()?))
            }
        }
    }
}

/// The pixel size of `spec`'s tile under `device`, and the scale its content draws at to fill it,
/// as resvg sizes it. `None` when the tile would be smaller than a pixel.
fn tile_size(spec: &PatternSpec, device: Matrix3x2) -> Option<((u32, u32), (f32, f32))> {
    let full = multiply(matrix(spec.transform), device);
    let mut scale = (full.M11.hypot(full.M12), full.M21.hypot(full.M22));
    let mut pixels = (
        (spec.rect.2 * scale.0).round(),
        (spec.rect.3 * scale.1).round(),
    );
    if pixels.0 < 1.0 || pixels.1 < 1.0 {
        return None;
    }
    if pixels.0 > MAX_TILE {
        scale.0 *= MAX_TILE / pixels.0;
        pixels.0 = MAX_TILE;
    }
    if pixels.1 > MAX_TILE {
        scale.1 *= MAX_TILE / pixels.1;
        pixels.1 = MAX_TILE;
    }
    Some(((pixels.0 as u32, pixels.1 as u32), scale))
}

/// Draws `spec`'s content into a `pixels` tile at `scale` and makes the brush that repeats it,
/// placed as resvg places its pattern pixmap.
fn pattern_tile(
    target: &ID2D1RenderTarget,
    spec: &mut PatternSpec,
    pixels: (u32, u32),
    scale: (f32, f32),
    opacity: f32,
) -> Result<ID2D1Brush> {
    unsafe {
        let tile = target.CreateCompatibleRenderTarget(
            Some(&D2D_SIZE_F {
                width: pixels.0 as f32,
                height: pixels.1 as f32,
            }),
            Some(&D2D_SIZE_U {
                width: pixels.0,
                height: pixels.1,
            }),
            None,
            D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE,
        )?;
        let surface: ID2D1RenderTarget = tile.cast()?;
        // Brushes belong to the target they were made on: this tile is a new one.
        spec.scene.release_target();
        surface.BeginDraw();
        surface.Clear(Some(&D2D1_COLOR_F::default()));
        let drawn = spec
            .scene
            .draw_under(&surface, matrix(Transform::from_scale(scale.0, scale.1)));
        surface.SetTransform(&Matrix3x2::identity());
        surface.EndDraw(None, None)?;
        drawn?;
        spec.scene.release_target();
        let bitmap = tile.GetBitmap()?;
        let place = spec
            .transform
            .pre_translate(spec.rect.0, spec.rect.1)
            .pre_scale(1.0 / scale.0, 1.0 / scale.1);
        let brush = target.CreateBitmapBrush(
            &bitmap,
            Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                extendModeX: D2D1_EXTEND_MODE_WRAP,
                extendModeY: D2D1_EXTEND_MODE_WRAP,
                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            }),
            Some(&D2D1_BRUSH_PROPERTIES {
                opacity,
                transform: matrix(place),
            }),
        )?;
        brush.cast()
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
    fn other_filters_blend_modes_and_images_stay_on_resvg() {
        // Break caught: a turbulence or lighting filter, a blended group, or a photo inside an
        // SVG drawn by Direct2D without its effect or its picture.
        for body in [
            r#"<filter id="t"><feTurbulence baseFrequency="0.05"/></filter><rect width="10" height="10" filter="url(#t)"/>"#,
            r#"<image width="2" height="2" href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg=="/>"#,
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
    fn gpu_pixels(tree: &usvg::Tree, size: (u32, u32)) -> Vec<u8> {
        gpu_region(tree, (size.0 as f32, size.1 as f32), (0, 0), size)
    }

    /// The `size` pixels at `origin` of `tree` drawn `picture` pixels large, as
    /// `svg::rasterize_region` takes them.
    fn gpu_region(
        tree: &usvg::Tree,
        picture: (f32, f32),
        origin: (u32, u32),
        (width, height): (u32, u32),
    ) -> Vec<u8> {
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
                .draw(&target, (-(origin.0 as f32), -(origin.1 as f32)), picture)
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

    /// How far Direct2D's pixels are from resvg's: the mean of each pixel's largest channel
    /// difference, and the share of pixels off by more than 64 of 255.
    fn difference(gpu: &[u8], cpu: &[u8]) -> (f64, f64) {
        let differences = gpu
            .chunks(4)
            .zip(cpu.chunks(4))
            .map(|(a, b)| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap())
            .collect::<Vec<_>>();
        let count = differences.len().max(1) as f64;
        (
            differences.iter().map(|&d| f64::from(d)).sum::<f64>() / count,
            differences.iter().filter(|&&d| d > 64).count() as f64 / count,
        )
    }

    /// Direct2D | resvg | their difference (red), side by side, as a PNG in `FASTPAD_VECTOR_PNGS`.
    fn save_comparison(name: &str, gpu: &[u8], cpu: &[u8], (width, height): (u32, u32)) {
        let mut row = Vec::with_capacity((width * height * 12) as usize);
        for y in 0..height as usize {
            let line = |pixels: &[u8]| {
                pixels[y * width as usize * 4..(y + 1) * width as usize * 4].to_vec()
            };
            row.extend(line(gpu));
            row.extend(line(cpu));
            for x in 0..width as usize {
                let at = (y * width as usize + x) * 4;
                let d = (0..4)
                    .map(|c| gpu[at + c].abs_diff(cpu[at + c]))
                    .max()
                    .unwrap();
                let d = (u32::from(d) * 4).min(255) as u8;
                row.extend([0, 0, d, 255]);
            }
        }
        save(name, &row, (width * 3, height));
    }

    /// What keeps `tree` from Direct2D, for the corpus report.
    fn not_drawable(tree: &usvg::Tree) -> Vec<String> {
        fn walk(group: &usvg::Group, out: &mut Vec<String>) {
            if group.blend_mode() != usvg::BlendMode::Normal {
                out.push(format!("blend {:?}", group.blend_mode()));
            }
            for filter in group.filters() {
                if !crate::image_view::effects::filter_drawable(filter) {
                    for primitive in filter.primitives() {
                        let kind = format!("{:?}", primitive.kind());
                        out.push(format!(
                            "filter {}",
                            kind.split('(').next().unwrap_or_default()
                        ));
                    }
                }
            }
            if let Some(clip) = group.clip_path()
                && !super::clip_drawable(clip)
            {
                out.push("nested clip".to_owned());
            }
            if let Some(mask) = group.mask() {
                walk(mask.root(), out);
            }
            for node in group.children() {
                match node {
                    usvg::Node::Group(group) => walk(group, out),
                    usvg::Node::Text(text) => walk(text.flattened(), out),
                    usvg::Node::Image(image) => match image.kind() {
                        usvg::ImageKind::SVG(tree) => walk(tree.root(), out),
                        _ => out.push("raster image".to_owned()),
                    },
                    usvg::Node::Path(path) => {
                        for paint in path
                            .fill()
                            .map(|f| f.paint())
                            .into_iter()
                            .chain(path.stroke().map(|s| s.paint()))
                        {
                            if !super::paint_drawable(paint) {
                                out.push("paint".to_owned());
                            }
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(tree.root(), &mut out);
        out.sort();
        out.dedup();
        out
    }

    /// Every `.svg` and `.fdraw` under the folders in `FASTPAD_VECTOR_CORPUS` (separated by `;`),
    /// drawn by Direct2D and by resvg, whole and as a 400% crop of the middle, and compared;
    /// diagrams in their light and dark looks. Prints what stays on resvg and why, and every
    /// picture that differs, saving those comparisons when `FASTPAD_VECTOR_PNGS` is set.
    /// `cargo test --release --lib vector_corpus -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn vector_corpus() {
        let Some(folders) = std::env::var_os("FASTPAD_VECTOR_CORPUS") else {
            return;
        };
        let mut files = Vec::new();
        for folder in folders.to_string_lossy().split(';') {
            let mut pending = vec![std::path::PathBuf::from(folder)];
            while let Some(folder) = pending.pop() {
                for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
                    let path = entry.path();
                    let skipped = path.file_name().is_some_and(|name| {
                        matches!(
                            name.to_str(),
                            Some("target" | "node_modules" | ".git" | ".worktrees")
                        )
                    });
                    if path.is_dir() && !skipped {
                        pending.push(path);
                    } else if matches!(
                        path.extension().and_then(|e| e.to_str()),
                        Some("svg" | "fdraw")
                    ) {
                        files.push(path);
                    }
                }
            }
        }
        files.sort();
        let (mut gpu_count, mut raster, mut unreadable, mut failures) =
            (0, Vec::new(), 0, Vec::new());
        for path in &files {
            let Ok(text) = std::fs::read_to_string(path) else {
                unreadable += 1;
                continue;
            };
            let diagram = path.extension().is_some_and(|e| e == "fdraw");
            let variants: Vec<(String, String)> = if diagram {
                [false, true]
                    .into_iter()
                    .filter_map(|dark| {
                        let drawn = crate::preview::fastdraw::draw(&text, Some(path), dark);
                        drawn
                            .show
                            .then(|| (if dark { "dark" } else { "light" }.to_owned(), drawn.svg))
                    })
                    .collect()
            } else {
                vec![(String::new(), text)]
            };
            for (variant, source) in variants {
                let label = format!("{} {variant}", path.display());
                let Ok(tree) = crate::preview::svg::parse(&source) else {
                    unreadable += 1;
                    continue;
                };
                if !drawable(&tree) {
                    raster.push(format!("{label} {:?}", not_drawable(&tree)));
                    continue;
                }
                gpu_count += 1;
                let natural = crate::preview::svg::natural_size(&tree);
                // The whole picture, at most 800 pixels a side.
                let fit = (800.0 / natural.0.max(natural.1) as f32).min(4.0);
                let whole = (
                    ((natural.0 as f32 * fit).round() as u32).max(1),
                    ((natural.1 as f32 * fit).round() as u32).max(1),
                );
                let picture = (whole.0 as f32, whole.1 as f32);
                let checks = [
                    ("whole", picture, (0, 0), whole),
                    (
                        "400%",
                        (picture.0 * 4.0, picture.1 * 4.0),
                        (
                            whole.0 * 2 - whole.0.min(400) / 2,
                            whole.1 * 2 - whole.1.min(400) / 2,
                        ),
                        (whole.0.min(400), whole.1.min(400)),
                    ),
                ];
                for (check, picture, origin, size) in checks {
                    let gpu = gpu_region(&tree, picture, origin, size);
                    let cpu = crate::preview::svg::rasterize_region(&tree, picture, origin, size)
                        .unwrap();
                    let (mean, far) = difference(&gpu, &cpu);
                    let differs = mean >= 3.0 || far >= 0.02;
                    if differs || std::env::var_os("FASTPAD_VECTOR_SAVE_ALL").is_some() {
                        let name = format!(
                            "{}-{variant}-{check}",
                            path.file_stem().unwrap().to_string_lossy()
                        )
                        .replace('%', "pct");
                        save_comparison(&name, &gpu, &cpu, size);
                    }
                    if differs {
                        failures.push(format!("{label} {check}: mean {mean:.2}, far {far:.4}"));
                    }
                }
            }
        }
        println!(
            "CORPUS {} files: {gpu_count} pictures drawn by Direct2D, {} left to resvg, {unreadable} not drawable at all",
            files.len(),
            raster.len()
        );
        for label in &raster {
            println!("RESVG {label}");
        }
        for failure in &failures {
            println!("DIFFERS {failure}");
        }
        assert!(failures.is_empty(), "{} pictures differ", failures.len());
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
            (
                "patterns",
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120">
                <defs><pattern id="dots" width="12" height="12" patternUnits="userSpaceOnUse"><circle cx="6" cy="6" r="1.5" fill="#888"/></pattern>
                <pattern id="checks" width="0.25" height="0.5" patternTransform="rotate(20)"><rect width="10" height="10" fill="#c33"/></pattern></defs>
                <rect width="200" height="120" fill="#fafaf7"/><rect width="200" height="120" fill="url(#dots)"/>
                <rect x="110" y="20" width="80" height="80" fill="url(#checks)" fill-opacity="0.7" stroke="#000"/></svg>"##,
            ),
            (
                "embedded-svg",
                // FastDraw places its icons as SVG images inside the diagram.
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120">
                <rect width="200" height="120" fill="#eef"/>
                <image x="20" y="20" width="80" height="80" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 18 18'%3E%3Cdefs%3E%3ClinearGradient id='a' x2='0' y2='1'%3E%3Cstop offset='0' stop-color='%2332d4f5'/%3E%3Cstop offset='1' stop-color='%23198ab3'/%3E%3C/linearGradient%3E%3C/defs%3E%3Crect x='1' y='3' width='16' height='12' rx='2' fill='url(%23a)'/%3E%3Ccircle cx='9' cy='9' r='3' fill='%23fff'/%3E%3C/svg%3E"/>
                <image x="120" y="30" width="60" height="30" preserveAspectRatio="xMidYMid meet" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'%3E%3Ccircle cx='5' cy='5' r='5' fill='%23e5484d'/%3E%3C/svg%3E"/></svg>"##,
            ),
            (
                "drop-shadow",
                // The drop shadow design tools export (Figma), as in VS Code's illustrations.
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120">
                <defs><filter id="s" x="10" y="10" width="120" height="100" filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
                <feFlood flood-opacity="0" result="BackgroundImageFix"/>
                <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
                <feOffset dy="4"/><feGaussianBlur stdDeviation="6"/>
                <feColorMatrix type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0.25 0"/>
                <feBlend mode="normal" in2="BackgroundImageFix" result="effect1_dropShadow"/>
                <feBlend mode="normal" in="SourceGraphic" in2="effect1_dropShadow" result="shape"/></filter>
                <filter id="d"><feDropShadow dx="3" dy="5" stdDeviation="3" flood-color="#036" flood-opacity="0.6"/></filter></defs>
                <rect width="200" height="120" fill="#f4f4f8"/>
                <g filter="url(#s)"><rect x="30" y="26" width="80" height="56" rx="8" fill="#fff"/></g>
                <circle cx="160" cy="60" r="24" fill="#e85" filter="url(#d)"/></svg>"##,
            ),
            (
                "blur-and-composite",
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120">
                <defs><filter id="b"><feGaussianBlur stdDeviation="4 1"/></filter>
                <filter id="c"><feFlood flood-color="#08f" result="f"/><feComposite in="f" in2="SourceGraphic" operator="in" result="tinted"/>
                <feMerge><feMergeNode in="tinted"/><feMergeNode in="SourceGraphic"/></feMerge></filter>
                <filter id="a"><feFlood flood-color="#f60" flood-opacity="0.8" result="f"/><feComposite in="SourceGraphic" in2="f" operator="arithmetic" k1="0.5" k2="0.7" k3="0.2" k4="0"/></filter>
                <filter id="h"><feColorMatrix type="hueRotate" values="90"/><feColorMatrix type="saturate" values="0.4"/></filter></defs>
                <rect x="10" y="10" width="50" height="40" fill="#c0c" filter="url(#b)"/>
                <path d="M80 10 h40 v40 z" fill="#333" filter="url(#c)"/>
                <rect x="140" y="10" width="50" height="40" fill="#09c" filter="url(#a)"/>
                <rect x="10" y="70" width="180" height="40" fill="#e33" filter="url(#h)"/></svg>"##,
            ),
            (
                "masks",
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120">
                <defs><linearGradient id="fade" x2="1"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></linearGradient>
                <mask id="l"><rect width="200" height="120" fill="url(#fade)"/></mask>
                <mask id="al" mask-type="alpha"><circle cx="150" cy="60" r="40" fill="#000" fill-opacity="0.5"/></mask>
                <mask id="r" x="20" y="20" width="60" height="30" maskUnits="userSpaceOnUse"><rect width="200" height="120" fill="#fff"/></mask>
                <mask id="soft" fill="white"><circle cx="40" cy="90" r="22"/></mask></defs>
                <rect width="200" height="120" fill="#ddd"/>
                <rect x="0" y="0" width="200" height="60" fill="#36c" mask="url(#l)"/>
                <rect x="100" y="20" width="100" height="80" fill="#c33" mask="url(#al)"/>
                <rect x="0" y="0" width="100" height="120" fill="#080" mask="url(#r)" opacity="0.7"/>
                <!-- A shape's own edges in a luminance mask: faint white masks faintly. -->
                <rect x="10" y="60" width="60" height="60" fill="#424242" mask="url(#soft)"/></svg>"##,
            ),
        ];
        // A real FastDraw diagram: its look paints the paper with a dot pattern.
        let diagram = crate::preview::fastdraw::draw(
            "a: node \"Orders API\" sub=\"/orders\" at 40,40 160x64
b: node \"Queue\" at 260,40 160x64
a -> b \"publish\"
",
            None,
            false,
        );
        assert!(diagram.show);
        let samples = samples
            .into_iter()
            .map(|(name, source)| (name, source.to_owned()))
            .chain([("fastdraw", diagram.svg)]);
        for (name, source) in samples {
            let tree = crate::preview::svg::parse(&source).unwrap();
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
#[cfg(test)]
#[test]
fn zz_device_context() {
    use windows::Win32::Graphics::Direct2D::Common::*;
    use windows::Win32::Graphics::Direct2D::*;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    let _com = crate::preview::images::ComScope::enter();
    let graphics = crate::preview::dwrite::Graphics::load().unwrap();
    unsafe {
        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
        let bitmap = wic
            .CreateBitmap(64, 64, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)
            .unwrap();
        let props = D2D1_RENDER_TARGET_PROPERTIES {
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
            .CreateWicBitmapRenderTarget(&bitmap, &props)
            .unwrap();
        println!("CTX wic: {:?}", target.cast::<ID2D1DeviceContext>().is_ok());
        let compat = target
            .CreateCompatibleRenderTarget(
                None,
                Some(&D2D_SIZE_U {
                    width: 8,
                    height: 8,
                }),
                None,
                D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE,
            )
            .unwrap();
        println!(
            "CTX compat: {:?}",
            compat.cast::<ID2D1DeviceContext>().is_ok()
        );
        let ctx: ID2D1DeviceContext = target.cast().unwrap();
        let blur = ctx.CreateEffect(&CLSID_D2D1GaussianBlur);
        println!("CTX blur effect: {:?}", blur.is_ok());
        for (name, id) in [
            ("alpha mask", CLSID_D2D1AlphaMask),
            ("lum2alpha", CLSID_D2D1LuminanceToAlpha),
            ("table", CLSID_D2D1TableTransfer),
            ("morph", CLSID_D2D1Morphology),
            ("tile", CLSID_D2D1Tile),
            ("turb", CLSID_D2D1Turbulence),
            ("convolve", CLSID_D2D1ConvolveMatrix),
            ("flood", CLSID_D2D1Flood),
            ("colormatrix", CLSID_D2D1ColorMatrix),
            ("blend", CLSID_D2D1Blend),
            ("composite", CLSID_D2D1Composite),
            ("arith", CLSID_D2D1ArithmeticComposite),
            ("crop", CLSID_D2D1Crop),
            ("affine", CLSID_D2D12DAffineTransform),
        ] {
            println!("CTX {name}: {:?}", ctx.CreateEffect(&id).is_ok());
        }
    }
}
