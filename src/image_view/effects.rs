//! SVG filters and masks as Direct2D effects, for `vector::Scene`. Each follows resvg's rules
//! step for step (its `filter` and `mask` modules), so the GPU picture matches the raster one:
//! the same regions in device pixels, blur and offsets scaled by the zoom, linearRGB conversions
//! by resvg's own tables, and results cropped where resvg clears them. Only the primitives listed
//! in `filter_drawable` are drawn; a tree with any other keeps the resvg path.

use resvg::tiny_skia::{IntRect, Transform};
use resvg::usvg;
use resvg::usvg::filter::{ColorInterpolation, ColorMatrixKind, CompositeOperator, Input, Kind};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_BLEND_MODE, D2D1_BLEND_MODE_COLOR, D2D1_BLEND_MODE_COLOR_BURN,
    D2D1_BLEND_MODE_COLOR_DODGE, D2D1_BLEND_MODE_DARKEN, D2D1_BLEND_MODE_DIFFERENCE,
    D2D1_BLEND_MODE_EXCLUSION, D2D1_BLEND_MODE_HARD_LIGHT, D2D1_BLEND_MODE_HUE,
    D2D1_BLEND_MODE_LIGHTEN, D2D1_BLEND_MODE_LUMINOSITY, D2D1_BLEND_MODE_MULTIPLY,
    D2D1_BLEND_MODE_OVERLAY, D2D1_BLEND_MODE_SATURATION, D2D1_BLEND_MODE_SCREEN,
    D2D1_BLEND_MODE_SOFT_LIGHT, D2D1_COMPOSITE_MODE, D2D1_COMPOSITE_MODE_SOURCE_ATOP,
    D2D1_COMPOSITE_MODE_SOURCE_IN, D2D1_COMPOSITE_MODE_SOURCE_OUT, D2D1_COMPOSITE_MODE_SOURCE_OVER,
    D2D1_COMPOSITE_MODE_XOR,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1AlphaMask, CLSID_D2D1ArithmeticComposite, CLSID_D2D1Blend, CLSID_D2D1ColorMatrix,
    CLSID_D2D1Composite, CLSID_D2D1Crop, CLSID_D2D1DirectionalBlur, CLSID_D2D1Flood,
    CLSID_D2D1GaussianBlur, CLSID_D2D1TableTransfer, CLSID_D2D12DAffineTransform,
    D2D1_PROPERTY_TYPE, D2D1_PROPERTY_TYPE_BLOB, D2D1_PROPERTY_TYPE_BOOL, D2D1_PROPERTY_TYPE_ENUM,
    D2D1_PROPERTY_TYPE_FLOAT, D2D1_PROPERTY_TYPE_MATRIX_3X2, D2D1_PROPERTY_TYPE_MATRIX_5X4,
    D2D1_PROPERTY_TYPE_VECTOR4, ID2D1DeviceContext, ID2D1Effect, ID2D1Image,
};
use windows::core::{GUID, Result};

/// Whether every primitive of `filter` is one Direct2D effects draw as resvg does.
pub(crate) fn filter_drawable(filter: &usvg::filter::Filter) -> bool {
    filter.primitives().iter().all(|primitive| {
        matches!(
            primitive.kind(),
            Kind::Blend(_)
                | Kind::ColorMatrix(_)
                | Kind::Composite(_)
                | Kind::DropShadow(_)
                | Kind::Flood(_)
                | Kind::GaussianBlur(_)
                | Kind::Merge(_)
                | Kind::Offset(_)
        ) && match primitive.kind() {
            Kind::Blend(blend) => {
                blend_mode(blend.mode()).is_some() || blend.mode() == usvg::BlendMode::Normal
            }
            _ => true,
        }
    })
}

/// How far, in device pixels, the filters' blurs and offsets reach past what they are given: the
/// part of the picture outside the view that can still show inside it.
pub(super) fn reach(filters: &[std::sync::Arc<usvg::filter::Filter>], scale: (f32, f32)) -> f32 {
    let scale = scale.0.max(scale.1);
    filters
        .iter()
        .flat_map(|filter| filter.primitives())
        .map(|primitive| match primitive.kind() {
            Kind::GaussianBlur(blur) => 3.0 * blur.std_dev_x().get().max(blur.std_dev_y().get()),
            Kind::DropShadow(shadow) => {
                3.0 * shadow.std_dev_x().get().max(shadow.std_dev_y().get())
                    + shadow.dx().abs().max(shadow.dy().abs())
            }
            Kind::Offset(offset) => offset.dx().abs().max(offset.dy().abs()),
            _ => 0.0,
        })
        .sum::<f32>()
        * scale
        + 2.0
}

/// A filter result: its image in layer pixels, the area it covers, and its colour space.
#[derive(Clone)]
struct Image {
    image: ID2D1Image,
    region: IntRect,
    space: ColorInterpolation,
}

/// Applies `filter` to `source`, a layer `size` pixels large, as resvg applies it to a layer
/// pixmap; `ts` takes the group's space to the layer's pixels. `None` when the filter shows
/// nothing (an empty region, or no primitives).
pub(super) fn apply_filter(
    context: &ID2D1DeviceContext,
    filter: &usvg::filter::Filter,
    ts: Transform,
    source: &ID2D1Image,
    size: (u32, u32),
) -> Result<Option<ID2D1Image>> {
    let graph = Graph { context };
    let Some(region) = filter
        .rect()
        .transform(ts)
        .map(|rect| rect.to_int_rect())
        .and_then(|region| fit(region, size))
    else {
        return Ok(None);
    };
    let mut results: Vec<(String, Image)> = Vec::new();
    for primitive in filter.primitives() {
        let mut subregion = primitive
            .rect()
            .transform(ts)
            .map(|rect| rect.to_int_rect())
            .unwrap_or(region);
        // As resvg: `feOffset` takes its input's area.
        if let Kind::Offset(offset) = primitive.kind()
            && let Input::Reference(name) = offset.input()
            && let Some((_, result)) = results.iter().rev().find(|(n, _)| n == name)
        {
            subregion = result.region;
        }
        let space = primitive.color_interpolation();
        let input = |input: &Input| graph.input(input, region, source, &results);
        let mut result = match primitive.kind() {
            Kind::Blend(blend) => {
                let first = graph.in_space(input(blend.input1())?, space)?;
                let second = graph.in_space(input(blend.input2())?, space)?;
                let image = match blend_mode(blend.mode()) {
                    Some(mode) => graph.blend(&second.image, &first.image, mode)?,
                    None => graph.composite(
                        &[second.image, first.image],
                        D2D1_COMPOSITE_MODE_SOURCE_OVER,
                    )?,
                };
                Image {
                    image: graph.crop(&image, region)?,
                    region,
                    space,
                }
            }
            Kind::Composite(composite) => {
                let first = graph.in_space(input(composite.input1())?, space)?;
                let second = graph.in_space(input(composite.input2())?, space)?;
                let image = match composite.operator() {
                    CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
                        graph.arithmetic(&second.image, &first.image, [k1, k2, k3, k4])?
                    }
                    operator => {
                        graph.composite(&[second.image, first.image], composite_mode(operator))?
                    }
                };
                Image {
                    image: graph.crop(&image, region)?,
                    region,
                    space,
                }
            }
            Kind::Merge(merge) => {
                let mut layers = Vec::new();
                for each in merge.inputs() {
                    layers.push(graph.in_space(input(each)?, space)?.image);
                }
                let image = if layers.is_empty() {
                    graph.flood([0.0; 4])?
                } else {
                    graph.composite(&layers, D2D1_COMPOSITE_MODE_SOURCE_OVER)?
                };
                Image {
                    image: graph.crop(&image, region)?,
                    region,
                    space,
                }
            }
            Kind::Flood(flood) => {
                let color = flood.color();
                let image = graph.flood([
                    f32::from(color.red) / 255.0,
                    f32::from(color.green) / 255.0,
                    f32::from(color.blue) / 255.0,
                    flood.opacity().get(),
                ])?;
                Image {
                    image: graph.crop(&image, region)?,
                    region,
                    space: ColorInterpolation::SRGB,
                }
            }
            Kind::GaussianBlur(blur) => {
                let given = input(blur.input())?;
                match std_dev(blur.std_dev_x().get(), blur.std_dev_y().get(), ts) {
                    None => given,
                    Some(deviation) => {
                        let given = graph.in_space(given, space)?;
                        let image = graph.blur(&given.image, deviation)?;
                        Image {
                            image: graph.crop(&image, region)?,
                            region,
                            space,
                        }
                    }
                }
            }
            Kind::Offset(offset) => {
                let given = input(offset.input())?;
                let (dx, dy) = scaled(offset.dx(), offset.dy(), ts);
                // resvg draws the shifted pixmap at whole pixels.
                let (dx, dy) = (dx as i32, dy as i32);
                if dx == 0 && dy == 0 {
                    given
                } else {
                    let image = graph.shift(&given.image, (dx as f32, dy as f32))?;
                    Image {
                        image: graph.crop(&image, region)?,
                        region,
                        space: given.space,
                    }
                }
            }
            Kind::ColorMatrix(matrix) => {
                let given = graph.in_space(input(matrix.input())?, space)?;
                let image = graph.color_matrix(&given.image, color_matrix(matrix.kind()))?;
                Image {
                    image: graph.crop(&image, region)?,
                    region,
                    space,
                }
            }
            Kind::DropShadow(shadow) => {
                let given = graph.in_space(input(shadow.input())?, space)?;
                let (dx, dy) = scaled(shadow.dx(), shadow.dy(), ts);
                let blurred = match std_dev(shadow.std_dev_x().get(), shadow.std_dev_y().get(), ts)
                {
                    Some(deviation) => graph.blur(&given.image, deviation)?,
                    None => given.image.clone(),
                };
                // The shadow's colour, as resvg treats it: sRGB, converted into the primitive's
                // space, at the shadow's alpha times its opacity.
                let color = shadow.color();
                let channel = |value: u8| match space {
                    ColorInterpolation::SRGB => f32::from(value) / 255.0,
                    ColorInterpolation::LinearRGB => {
                        f32::from(SRGB_TO_LINEAR[usize::from(value)]) / 255.0
                    }
                };
                let opacity = shadow.opacity().get();
                #[rustfmt::skip]
                let tint = [
                    0.0, 0.0, 0.0, 0.0, channel(color.red),
                    0.0, 0.0, 0.0, 0.0, channel(color.green),
                    0.0, 0.0, 0.0, 0.0, channel(color.blue),
                    0.0, 0.0, 0.0, opacity, 0.0,
                ];
                let colored = graph.color_matrix(&blurred, tint)?;
                let moved = graph.shift(&colored, (dx as i32 as f32, dy as i32 as f32))?;
                let image =
                    graph.composite(&[moved, given.image], D2D1_COMPOSITE_MODE_SOURCE_OVER)?;
                Image {
                    image: graph.crop(&image, region)?,
                    region,
                    space,
                }
            }
            _ => return Ok(None),
        };
        if region != subregion
            && !matches!(primitive.kind(), Kind::Offset(_))
            && let Some(clip) = subregion.intersect(&region)
        {
            result.image = graph.crop(&result.image, clip)?;
        }
        if matches!(primitive.kind(), Kind::Offset(_)) {
            result.region = subregion;
        }
        results.push((primitive.result().to_owned(), result));
    }
    let Some((_, last)) = results.pop() else {
        return Ok(None);
    };
    let last = graph.in_space(last, ColorInterpolation::SRGB)?;
    Ok(Some(graph.crop(&last.image, region)?))
}

/// Multiplies `content` by `mask`, an image of the mask's content already clipped to its
/// rectangle: by its luminance, or for an alpha mask by its alpha.
pub(super) fn apply_mask(
    context: &ID2D1DeviceContext,
    content: &ID2D1Image,
    mask: &ID2D1Image,
    kind: usvg::MaskType,
) -> Result<ID2D1Image> {
    let graph = Graph { context };
    let alpha = match kind {
        // An SVG mask's luminance is of premultiplied colour: a faint white edge masks faintly.
        // Direct2D's luminance-to-alpha effect works on unpremultiplied colour, so the matrix
        // runs on the premultiplied values as they are.
        #[rustfmt::skip]
        usvg::MaskType::Luminance => graph.color_matrix_as(mask, [
            0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0,
            0.2125, 0.7154, 0.0721, 0.0, 0.0,
        ], AS_STORED)?,
        usvg::MaskType::Alpha => mask.clone(),
    };
    graph.effect(&CLSID_D2D1AlphaMask, &[content.clone(), alpha])
}

/// `region` within a layer `size` pixels large.
fn fit(region: IntRect, size: (u32, u32)) -> Option<IntRect> {
    region.intersect(&IntRect::from_xywh(0, 0, size.0, size.1)?)
}

fn scaled(x: f32, y: f32, ts: Transform) -> (f32, f32) {
    let (sx, sy) = ts.get_scale();
    (x * sx, y * sy)
}

/// The blur's deviations in pixels, as resvg resolves them; `None` for no blur at all.
fn std_dev(x: f32, y: f32, ts: Transform) -> Option<(f32, f32)> {
    let (mut x, mut y) = scaled(x, y, ts);
    if x.abs() < 1e-6 && y.abs() < 1e-6 {
        return None;
    }
    // resvg ignores tiny deviations.
    if x < 0.05 {
        x = 0.0;
    }
    if y < 0.05 {
        y = 0.0;
    }
    Some((x, y))
}

fn blend_mode(mode: usvg::BlendMode) -> Option<D2D1_BLEND_MODE> {
    use usvg::BlendMode as B;
    Some(match mode {
        B::Normal => return None,
        B::Multiply => D2D1_BLEND_MODE_MULTIPLY,
        B::Screen => D2D1_BLEND_MODE_SCREEN,
        B::Overlay => D2D1_BLEND_MODE_OVERLAY,
        B::Darken => D2D1_BLEND_MODE_DARKEN,
        B::Lighten => D2D1_BLEND_MODE_LIGHTEN,
        B::ColorDodge => D2D1_BLEND_MODE_COLOR_DODGE,
        B::ColorBurn => D2D1_BLEND_MODE_COLOR_BURN,
        B::HardLight => D2D1_BLEND_MODE_HARD_LIGHT,
        B::SoftLight => D2D1_BLEND_MODE_SOFT_LIGHT,
        B::Difference => D2D1_BLEND_MODE_DIFFERENCE,
        B::Exclusion => D2D1_BLEND_MODE_EXCLUSION,
        B::Hue => D2D1_BLEND_MODE_HUE,
        B::Saturation => D2D1_BLEND_MODE_SATURATION,
        B::Color => D2D1_BLEND_MODE_COLOR,
        B::Luminosity => D2D1_BLEND_MODE_LUMINOSITY,
    })
}

fn composite_mode(operator: CompositeOperator) -> D2D1_COMPOSITE_MODE {
    match operator {
        CompositeOperator::In => D2D1_COMPOSITE_MODE_SOURCE_IN,
        CompositeOperator::Out => D2D1_COMPOSITE_MODE_SOURCE_OUT,
        CompositeOperator::Atop => D2D1_COMPOSITE_MODE_SOURCE_ATOP,
        CompositeOperator::Xor => D2D1_COMPOSITE_MODE_XOR,
        CompositeOperator::Over | CompositeOperator::Arithmetic { .. } => {
            D2D1_COMPOSITE_MODE_SOURCE_OVER
        }
    }
}

/// An SVG colour matrix (rows R, G, B, A of five) for `kind`, as resvg computes it.
fn color_matrix(kind: &ColorMatrixKind) -> [f32; 20] {
    let rgb = |m: [f32; 9]| {
        [
            m[0], m[1], m[2], 0.0, 0.0, m[3], m[4], m[5], 0.0, 0.0, m[6], m[7], m[8], 0.0, 0.0,
            0.0, 0.0, 0.0, 1.0, 0.0,
        ]
    };
    match kind {
        ColorMatrixKind::Matrix(values) => {
            let mut matrix = [0.0; 20];
            for (slot, value) in matrix.iter_mut().zip(values) {
                *slot = *value;
            }
            matrix
        }
        ColorMatrixKind::Saturate(value) => {
            let v = value.get().max(0.0);
            rgb([
                0.213 + 0.787 * v,
                0.715 - 0.715 * v,
                0.072 - 0.072 * v,
                0.213 - 0.213 * v,
                0.715 + 0.285 * v,
                0.072 - 0.072 * v,
                0.213 - 0.213 * v,
                0.715 - 0.715 * v,
                0.072 + 0.928 * v,
            ])
        }
        ColorMatrixKind::HueRotate(angle) => {
            let (a2, a1) = angle.to_radians().sin_cos();
            rgb([
                0.213 + 0.787 * a1 - 0.213 * a2,
                0.715 - 0.715 * a1 - 0.715 * a2,
                0.072 - 0.072 * a1 + 0.928 * a2,
                0.213 - 0.213 * a1 + 0.143 * a2,
                0.715 + 0.285 * a1 + 0.140 * a2,
                0.072 - 0.072 * a1 - 0.283 * a2,
                0.213 - 0.213 * a1 - 0.787 * a2,
                0.715 - 0.715 * a1 + 0.715 * a2,
                0.072 + 0.928 * a1 + 0.072 * a2,
            ])
        }
        #[rustfmt::skip]
        ColorMatrixKind::LuminanceToAlpha => [
            0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0,
            0.2125, 0.7154, 0.0721, 0.0, 0.0,
        ],
    }
}

/// Direct2D's colour matrix alpha modes: `D2D1_COLORMATRIX_ALPHA_MODE_PREMULTIPLIED` unpremultiplies,
/// applies the matrix and premultiplies again; `..._STRAIGHT` applies it to the values as stored.
const UNPREMULTIPLIED: u32 = 1;
const AS_STORED: u32 = 2;

/// Builds effect graphs on one device context.
struct Graph<'a> {
    context: &'a ID2D1DeviceContext,
}

impl Graph<'_> {
    fn input(
        &self,
        input: &Input,
        region: IntRect,
        source: &ID2D1Image,
        results: &[(String, Image)],
    ) -> Result<Image> {
        match input {
            Input::SourceGraphic => Ok(Image {
                image: self.crop(source, region)?,
                region,
                space: ColorInterpolation::SRGB,
            }),
            Input::SourceAlpha => {
                #[rustfmt::skip]
                let alpha = [
                    0.0, 0.0, 0.0, 0.0, 0.0,
                    0.0, 0.0, 0.0, 0.0, 0.0,
                    0.0, 0.0, 0.0, 0.0, 0.0,
                    0.0, 0.0, 0.0, 1.0, 0.0,
                ];
                let image = self.color_matrix(&self.crop(source, region)?, alpha)?;
                Ok(Image {
                    image,
                    region,
                    space: ColorInterpolation::SRGB,
                })
            }
            Input::Reference(name) => match results.iter().rev().find(|(n, _)| n == name) {
                Some((_, image)) => Ok(image.clone()),
                None => self.input(&Input::SourceGraphic, region, source, results),
            },
        }
    }

    fn in_space(&self, image: Image, space: ColorInterpolation) -> Result<Image> {
        if image.space == space {
            return Ok(image);
        }
        let table: &[u8; 256] = match space {
            ColorInterpolation::LinearRGB => &SRGB_TO_LINEAR,
            ColorInterpolation::SRGB => &LINEAR_TO_SRGB,
        };
        // resvg converts unpremultiplied channels by table, alpha untouched.
        let region = image.region;
        // The transfer effects work on unpremultiplied channels themselves.
        let converted = self.make(&CLSID_D2D1TableTransfer, &[image.image])?;
        let values = table
            .iter()
            .map(|&v| f32::from(v) / 255.0)
            .collect::<Vec<_>>();
        let blob = floats(&values);
        unsafe {
            for (table, disable) in [(0, 1), (2, 3), (4, 5)] {
                set(&converted, table, D2D1_PROPERTY_TYPE_BLOB, &blob)?;
                set(
                    &converted,
                    disable,
                    D2D1_PROPERTY_TYPE_BOOL,
                    &0_u32.to_ne_bytes(),
                )?;
            }
            set(&converted, 7, D2D1_PROPERTY_TYPE_BOOL, &1_u32.to_ne_bytes())?;
            set(&converted, 8, D2D1_PROPERTY_TYPE_BOOL, &1_u32.to_ne_bytes())?;
        }
        let image = output(&converted)?;
        Ok(Image {
            image,
            region,
            space,
        })
    }

    fn effect(&self, id: &GUID, inputs: &[ID2D1Image]) -> Result<ID2D1Image> {
        let effect = self.make(id, inputs)?;
        output(&effect)
    }

    fn make(&self, id: &GUID, inputs: &[ID2D1Image]) -> Result<ID2D1Effect> {
        let effect = unsafe { self.context.CreateEffect(id)? };
        unsafe {
            if inputs.len() != 1 {
                effect.SetInputCount(inputs.len() as u32)?;
            }
            for (index, input) in inputs.iter().enumerate() {
                effect.SetInput(index as u32, input, false);
            }
        }
        Ok(effect)
    }

    fn crop(&self, image: &ID2D1Image, rect: IntRect) -> Result<ID2D1Image> {
        let effect = self.make(&CLSID_D2D1Crop, std::slice::from_ref(image))?;
        let bounds = [
            rect.x() as f32,
            rect.y() as f32,
            (rect.x() + rect.width() as i32) as f32,
            (rect.y() + rect.height() as i32) as f32,
        ];
        unsafe { set(&effect, 0, D2D1_PROPERTY_TYPE_VECTOR4, &floats(&bounds))? };
        output(&effect)
    }

    /// `color` straight (unpremultiplied), as SVG gives it.
    fn flood(&self, color: [f32; 4]) -> Result<ID2D1Image> {
        let effect = self.make(&CLSID_D2D1Flood, &[])?;
        // Direct2D's flood takes a premultiplied colour.
        let [r, g, b, a] = color;
        let color = [r * a, g * a, b * a, a];
        unsafe { set(&effect, 0, D2D1_PROPERTY_TYPE_VECTOR4, &floats(&color))? };
        output(&effect)
    }

    fn blur(&self, image: &ID2D1Image, (x, y): (f32, f32)) -> Result<ID2D1Image> {
        if (x - y).abs() < 1e-3 {
            let effect = self.make(&CLSID_D2D1GaussianBlur, std::slice::from_ref(image))?;
            unsafe { set(&effect, 0, D2D1_PROPERTY_TYPE_FLOAT, &x.to_ne_bytes())? };
            return output(&effect);
        }
        let mut image = image.clone();
        for (deviation, angle) in [(x, 0.0_f32), (y, 90.0)] {
            if deviation <= 0.0 {
                continue;
            }
            let effect = self.make(&CLSID_D2D1DirectionalBlur, &[image])?;
            unsafe {
                set(
                    &effect,
                    0,
                    D2D1_PROPERTY_TYPE_FLOAT,
                    &deviation.to_ne_bytes(),
                )?;
                set(&effect, 1, D2D1_PROPERTY_TYPE_FLOAT, &angle.to_ne_bytes())?;
            }
            image = output(&effect)?;
        }
        Ok(image)
    }

    fn shift(&self, image: &ID2D1Image, (dx, dy): (f32, f32)) -> Result<ID2D1Image> {
        let effect = self.make(&CLSID_D2D12DAffineTransform, std::slice::from_ref(image))?;
        let matrix = [1.0, 0.0, 0.0, 1.0, dx, dy];
        unsafe {
            // Nearest neighbour: whole-pixel shifts stay exact.
            set(&effect, 0, D2D1_PROPERTY_TYPE_ENUM, &0_u32.to_ne_bytes())?;
            set(&effect, 2, D2D1_PROPERTY_TYPE_MATRIX_3X2, &floats(&matrix))?;
        }
        output(&effect)
    }

    /// `svg` is an SVG matrix: rows R, G, B, A of five, the fifth an offset in 0..1. Applied to
    /// unpremultiplied colour, as resvg applies `feColorMatrix`.
    fn color_matrix(&self, image: &ID2D1Image, svg: [f32; 20]) -> Result<ID2D1Image> {
        self.color_matrix_as(image, svg, UNPREMULTIPLIED)
    }

    /// `color_matrix` in Direct2D's alpha `mode`: `UNPREMULTIPLIED`, or `AS_STORED` to apply the
    /// matrix to the premultiplied values themselves.
    fn color_matrix_as(&self, image: &ID2D1Image, svg: [f32; 20], mode: u32) -> Result<ID2D1Image> {
        let effect = self.make(&CLSID_D2D1ColorMatrix, std::slice::from_ref(image))?;
        // Direct2D multiplies a row [r g b a 1] by a 5×4 matrix: SVG's, transposed.
        let mut d2d = [0.0_f32; 20];
        for output_channel in 0..4 {
            for input in 0..5 {
                d2d[input * 4 + output_channel] = svg[output_channel * 5 + input];
            }
        }
        unsafe {
            set(&effect, 0, D2D1_PROPERTY_TYPE_MATRIX_5X4, &floats(&d2d))?;
            set(&effect, 1, D2D1_PROPERTY_TYPE_ENUM, &mode.to_ne_bytes())?;
            set(&effect, 2, D2D1_PROPERTY_TYPE_BOOL, &1_u32.to_ne_bytes())?;
        }
        output(&effect)
    }

    /// `destination` under `source` in `mode`.
    fn blend(
        &self,
        destination: &ID2D1Image,
        source: &ID2D1Image,
        mode: D2D1_BLEND_MODE,
    ) -> Result<ID2D1Image> {
        let effect = self.make(&CLSID_D2D1Blend, &[destination.clone(), source.clone()])?;
        unsafe {
            set(
                &effect,
                0,
                D2D1_PROPERTY_TYPE_ENUM,
                &(mode.0 as u32).to_ne_bytes(),
            )?
        };
        output(&effect)
    }

    /// `layers` composited bottom first with `mode`.
    fn composite(&self, layers: &[ID2D1Image], mode: D2D1_COMPOSITE_MODE) -> Result<ID2D1Image> {
        let effect = self.make(&CLSID_D2D1Composite, layers)?;
        unsafe {
            set(
                &effect,
                0,
                D2D1_PROPERTY_TYPE_ENUM,
                &(mode.0 as u32).to_ne_bytes(),
            )?
        };
        output(&effect)
    }

    /// SVG's `k1·i1·i2 + k2·i1 + k3·i2 + k4`, with `destination` as i2 and `source` as i1.
    fn arithmetic(
        &self,
        destination: &ID2D1Image,
        source: &ID2D1Image,
        k: [f32; 4],
    ) -> Result<ID2D1Image> {
        let effect = self.make(
            &CLSID_D2D1ArithmeticComposite,
            &[destination.clone(), source.clone()],
        )?;
        // Direct2D: C1·dst·src + C2·dst + C3·src + C4.
        let coefficients = [k[0], k[2], k[1], k[3]];
        unsafe {
            set(
                &effect,
                0,
                D2D1_PROPERTY_TYPE_VECTOR4,
                &floats(&coefficients),
            )?;
            set(&effect, 1, D2D1_PROPERTY_TYPE_BOOL, &1_u32.to_ne_bytes())?;
        }
        output(&effect)
    }
}

fn output(effect: &ID2D1Effect) -> Result<ID2D1Image> {
    unsafe { effect.GetOutput() }
}

unsafe fn set(
    effect: &ID2D1Effect,
    index: u32,
    kind: D2D1_PROPERTY_TYPE,
    data: &[u8],
) -> Result<()> {
    unsafe { effect.SetValue(index, kind, data) }
}

fn floats(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

/// resvg's sRGB → linearRGB table (librsvg's), for unpremultiplied channels.
#[rustfmt::skip]
pub(super) const SRGB_TO_LINEAR: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   1,   1,   1,   1,   1,   1,   1,   1,   1,
      1,   1,   2,   2,   2,   2,   2,   2,   2,   2,   3,   3,   3,   3,   3,   3,
      4,   4,   4,   4,   4,   5,   5,   5,   5,   6,   6,   6,   6,   7,   7,   7,
      8,   8,   8,   8,   9,   9,   9,  10,  10,  10,  11,  11,  12,  12,  12,  13,
     13,  13,  14,  14,  15,  15,  16,  16,  17,  17,  17,  18,  18,  19,  19,  20,
     20,  21,  22,  22,  23,  23,  24,  24,  25,  25,  26,  27,  27,  28,  29,  29,
     30,  30,  31,  32,  32,  33,  34,  35,  35,  36,  37,  37,  38,  39,  40,  41,
     41,  42,  43,  44,  45,  45,  46,  47,  48,  49,  50,  51,  51,  52,  53,  54,
     55,  56,  57,  58,  59,  60,  61,  62,  63,  64,  65,  66,  67,  68,  69,  70,
     71,  72,  73,  74,  76,  77,  78,  79,  80,  81,  82,  84,  85,  86,  87,  88,
     90,  91,  92,  93,  95,  96,  97,  99, 100, 101, 103, 104, 105, 107, 108, 109,
    111, 112, 114, 115, 116, 118, 119, 121, 122, 124, 125, 127, 128, 130, 131, 133,
    134, 136, 138, 139, 141, 142, 144, 146, 147, 149, 151, 152, 154, 156, 157, 159,
    161, 163, 164, 166, 168, 170, 171, 173, 175, 177, 179, 181, 183, 184, 186, 188,
    190, 192, 194, 196, 198, 200, 202, 204, 206, 208, 210, 212, 214, 216, 218, 220,
    222, 224, 226, 229, 231, 233, 235, 237, 239, 242, 244, 246, 248, 250, 253, 255,
];

/// resvg's linearRGB → sRGB table.
#[rustfmt::skip]
pub(super) const LINEAR_TO_SRGB: [u8; 256] = [
      0,  13,  22,  28,  34,  38,  42,  46,  50,  53,  56,  59,  61,  64,  66,  69,
     71,  73,  75,  77,  79,  81,  83,  85,  86,  88,  90,  92,  93,  95,  96,  98,
     99, 101, 102, 104, 105, 106, 108, 109, 110, 112, 113, 114, 115, 117, 118, 119,
    120, 121, 122, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136,
    137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 148, 149, 150, 151,
    152, 153, 154, 155, 155, 156, 157, 158, 159, 159, 160, 161, 162, 163, 163, 164,
    165, 166, 167, 167, 168, 169, 170, 170, 171, 172, 173, 173, 174, 175, 175, 176,
    177, 178, 178, 179, 180, 180, 181, 182, 182, 183, 184, 185, 185, 186, 187, 187,
    188, 189, 189, 190, 190, 191, 192, 192, 193, 194, 194, 195, 196, 196, 197, 197,
    198, 199, 199, 200, 200, 201, 202, 202, 203, 203, 204, 205, 205, 206, 206, 207,
    208, 208, 209, 209, 210, 210, 211, 212, 212, 213, 213, 214, 214, 215, 215, 216,
    216, 217, 218, 218, 219, 219, 220, 220, 221, 221, 222, 222, 223, 223, 224, 224,
    225, 226, 226, 227, 227, 228, 228, 229, 229, 230, 230, 231, 231, 232, 232, 233,
    233, 234, 234, 235, 235, 236, 236, 237, 237, 238, 238, 238, 239, 239, 240, 240,
    241, 241, 242, 242, 243, 243, 244, 244, 245, 245, 246, 246, 246, 247, 247, 248,
    248, 249, 249, 250, 250, 251, 251, 251, 252, 252, 253, 253, 254, 254, 255, 255,
];
