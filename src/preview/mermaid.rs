//! Mermaid diagrams: a fenced code block whose language is `mermaid` draws as a diagram. The
//! source is parsed, laid out and written as SVG by `mermaid-rs-renderer`, in Rust and without a
//! browser, then rasterized by resvg like an SVG image. Both run on the image worker thread; the
//! preview shows the code block until the diagram is ready.
//!
//! Diagrams take their colors and font from the preview, so they match every theme, high
//! contrast included.

use crate::preview::colors::PreviewColors;
use crate::preview::images::{DecodedImage, MAX_IMAGE_PIXELS};
use mermaid_rs_renderer::{RenderOptions, Theme};
use std::sync::Arc;

/// Larger sources stay a code block: layout is superlinear in the node count.
pub const MAX_DIAGRAM_SOURCE: usize = 64 * 1024;

/// Whether a fenced code block's language makes it a diagram.
pub fn is_mermaid(language: &str) -> bool {
    language.eq_ignore_ascii_case("mermaid")
}

/// What a diagram looks like apart from its source: the preview's colors and font.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiagramStyle {
    colors: PreviewColors,
    font_family: String,
    /// The font size in DIPs, as `f32` bits so the style hashes.
    font_size: u32,
}

impl DiagramStyle {
    /// `body_family` and `body_size` are the preview's body font; diagram labels are set a
    /// little smaller, as Mermaid sets them beside 16 px body text.
    pub fn new(colors: PreviewColors, body_family: &str, body_size: f32) -> Self {
        Self {
            colors,
            font_family: body_family.to_owned(),
            font_size: (body_size * 0.875).max(1.0).to_bits(),
        }
    }

    fn font_size(&self) -> f32 {
        f32::from_bits(self.font_size)
    }
}

/// One diagram to draw: its source in one style. Cheap to clone; equal keys draw the same pixels.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiagramKey(Arc<(String, DiagramStyle)>);

impl DiagramKey {
    pub fn new(source: &str, style: &DiagramStyle) -> Self {
        Self(Arc::new((source.to_owned(), style.clone())))
    }

    pub fn source(&self) -> &str {
        &self.0.0
    }

    pub fn style(&self) -> &DiagramStyle {
        &self.0.1
    }
}

/// Renders `key` to premultiplied BGRA pixels. The natural size is the diagram's size in DIPs;
/// the pixels are `scale` device pixels per DIP, at most `max_width` wide (0 = no limit), so text
/// stays sharp at any DPI. Errors are one line, for the preview to show above the code.
pub fn render(key: &DiagramKey, max_width: u32, scale: f32) -> Result<DecodedImage, String> {
    let source = key.source();
    if source.len() > MAX_DIAGRAM_SOURCE {
        return Err("the diagram is larger than 64 KB".to_owned());
    }
    if source.trim().is_empty() {
        return Err("the diagram is empty".to_owned());
    }
    let options = RenderOptions {
        theme: theme(key.style()),
        ..RenderOptions::default()
    };
    let svg = mermaid_rs_renderer::render_with_options(source, options)
        .map_err(|error| first_line(&error.to_string()))?;
    let tree = crate::preview::svg::parse(&svg).map_err(|error| error.to_string())?;
    let (natural_width, natural_height) = crate::preview::svg::natural_size(&tree);
    let (width, height) = pixel_size((natural_width, natural_height), max_width, scale)
        .ok_or_else(|| "the diagram is larger than 64 megapixels".to_owned())?;
    let pixels =
        crate::preview::svg::rasterize(&tree, (width, height)).map_err(|e| e.to_string())?;
    Ok(DecodedImage {
        width,
        height,
        natural_width,
        natural_height,
        pixels,
    })
}

/// The raster size for a diagram `natural` DIPs large at `scale`, no wider than `max_width`
/// (0 = no limit); `None` when even one pixel per DIP would exceed the pixel limit.
pub(crate) fn pixel_size(natural: (u32, u32), max_width: u32, scale: f32) -> Option<(u32, u32)> {
    let (natural_width, natural_height) = (natural.0.max(1) as f32, natural.1.max(1) as f32);
    if u64::from(natural.0) * u64::from(natural.1) > MAX_IMAGE_PIXELS {
        return None;
    }
    let mut scale = if scale.is_finite() {
        scale.max(1.0)
    } else {
        1.0
    };
    if max_width > 0 {
        scale = scale.min(max_width as f32 / natural_width);
    }
    // Keep a high-DPI raster within the pixel limit.
    let limit = (MAX_IMAGE_PIXELS as f32 / (natural_width * natural_height)).sqrt();
    scale = scale.min(limit);
    Some((
        ((natural_width * scale).round() as u32).max(1),
        ((natural_height * scale).round() as u32).max(1),
    ))
}

fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default().trim();
    let mut line = if line.is_empty() {
        "the diagram could not be drawn".to_owned()
    } else {
        line.to_owned()
    };
    if line.chars().count() > 200 {
        line = line.chars().take(199).collect::<String>() + "…";
    }
    line
}

/// `#RRGGBB` for a `COLORREF` (`0x00BBGGRR`).
fn hex(color: u32) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        color & 0xFF,
        (color >> 8) & 0xFF,
        (color >> 16) & 0xFF
    )
}

/// `from` moved `amount` (0 to 1) of the way to `to`, per channel.
pub(crate) fn mix(from: u32, to: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let a = ((from >> shift) & 0xFF) as f32;
        let b = ((to >> shift) & 0xFF) as f32;
        ((a + (b - a) * amount).round() as u32).min(255) << shift
    };
    channel(0) | channel(8) | channel(16)
}

/// The `index`th pie slice: distinct hues, so neighbors never look alike, softened a quarter of the
/// way toward `background` so they sit in the theme. Repeats after ten slices.
fn pie_color(background: u32, index: usize) -> u32 {
    use crate::languages::rgb;
    const HUES: [u32; 10] = [
        rgb(0x4E, 0x79, 0xA7),
        rgb(0xF2, 0x8E, 0x2B),
        rgb(0x59, 0xA1, 0x4F),
        rgb(0xE1, 0x57, 0x59),
        rgb(0x76, 0xB7, 0xB2),
        rgb(0xED, 0xC9, 0x48),
        rgb(0xB0, 0x7A, 0xA1),
        rgb(0xFF, 0x9D, 0xA7),
        rgb(0x9C, 0x75, 0x5F),
        rgb(0xBA, 0xB0, 0xAC),
    ];
    mix(HUES[index % HUES.len()], background, 0.25)
}

/// Mermaid's "modern" theme recolored with the preview's roles: nodes on the code background,
/// borders and lines in the border and muted colors, text in the text color.
fn theme(style: &DiagramStyle) -> Theme {
    let colors = &style.colors;
    let text = hex(colors.text);
    let background = hex(colors.background);
    let node = hex(colors.code_background);
    let node_border = hex(mix(colors.border, colors.muted, 0.35));
    let line = hex(colors.muted);
    let secondary = hex(mix(colors.code_background, colors.border, 0.5));
    let mut theme = Theme::modern();
    theme.font_family = format!(
        "\"{}\", \"Segoe UI\", sans-serif",
        style.font_family.replace('"', "")
    );
    theme.font_size = style.font_size();
    theme.background = background.clone();
    theme.primary_color = node.clone();
    theme.primary_text_color = text.clone();
    theme.primary_border_color = node_border.clone();
    theme.line_color = line.clone();
    theme.secondary_color = secondary.clone();
    theme.tertiary_color = background.clone();
    theme.edge_label_background = background.clone();
    theme.cluster_background = hex(colors.table_stripe);
    theme.cluster_border = hex(colors.border);
    theme.sequence_actor_fill = node;
    theme.sequence_actor_border = node_border.clone();
    theme.sequence_actor_line = line;
    theme.sequence_note_fill = hex(colors.mark);
    theme.sequence_note_border = node_border.clone();
    theme.sequence_activation_fill = secondary;
    theme.sequence_activation_border = node_border;
    theme.text_color = text.clone();
    theme.pie_colors = std::array::from_fn(|index| hex(pie_color(colors.background, index)));
    theme.pie_title_text_color = text.clone();
    theme.pie_section_text_color = text.clone();
    theme.pie_legend_text_color = text.clone();
    theme.pie_stroke_color = background.clone();
    theme.pie_outer_stroke_color = hex(colors.border);
    theme.git_commit_label_color = text.clone();
    theme.git_commit_label_background = background.clone();
    theme.git_tag_label_color = text;
    theme.git_tag_label_background = background;
    theme
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::rgb;

    fn colors(dark: bool) -> PreviewColors {
        use crate::platform::theme::Theme;
        crate::preview::colors::preview_colors(if dark { Theme::Dark } else { Theme::Light }, false)
    }

    fn style() -> DiagramStyle {
        DiagramStyle::new(colors(false), "Segoe UI", 16.0)
    }

    fn pixel(image: &DecodedImage, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * image.width + x) * 4) as usize;
        image.pixels[offset..offset + 4].try_into().unwrap()
    }

    #[test]
    fn only_the_mermaid_language_is_a_diagram() {
        assert!(is_mermaid("mermaid"));
        assert!(is_mermaid("Mermaid"));
        assert!(!is_mermaid("mmd"));
        assert!(!is_mermaid(""));
    }

    #[test]
    fn a_flowchart_renders_at_its_natural_size_times_the_scale() {
        let key = DiagramKey::new("flowchart LR\n  A[Start] --> B[End]\n", &style());
        let image = render(&key, 0, 1.0).unwrap();
        assert!(image.natural_width > image.natural_height);
        assert_eq!(
            (image.width, image.height),
            (image.natural_width, image.natural_height)
        );
        assert_eq!(
            image.pixels.len(),
            (image.width * image.height * 4) as usize
        );
        let sharp = render(&key, 0, 2.0).unwrap();
        assert_eq!(sharp.natural_width, image.natural_width);
        assert!(sharp.width.abs_diff(2 * image.natural_width) <= 1);
        // The pane width caps the raster, never the natural size layout uses.
        let narrow = render(&key, 50, 2.0).unwrap();
        assert_eq!(narrow.width, 50);
        assert_eq!(narrow.natural_width, image.natural_width);
    }

    #[test]
    fn the_background_is_the_preview_background() {
        let mut colors = colors(true);
        colors.background = rgb(0x20, 0x40, 0x60);
        let key = DiagramKey::new(
            "flowchart TD\n  A --> B\n",
            &DiagramStyle::new(colors, "Segoe UI", 16.0),
        );
        let image = render(&key, 0, 1.0).unwrap();
        // BGRA, opaque.
        assert_eq!(pixel(&image, 0, 0), [0x60, 0x40, 0x20, 0xFF]);
    }

    #[test]
    fn every_common_diagram_type_renders() {
        let sources = [
            "sequenceDiagram\n  Alice->>John: Hello\n  John-->>Alice: Hi",
            "classDiagram\n  Animal <|-- Duck\n  Animal : +int age",
            "stateDiagram-v2\n  [*] --> Still\n  Still --> [*]",
            "erDiagram\n  CUSTOMER ||--o{ ORDER : places",
            "pie title Pets\n  \"Dogs\" : 386\n  \"Cats\" : 85",
            "gantt\n  dateFormat YYYY-MM-DD\n  section S\n  A task :a1, 2014-01-01, 30d",
            "journey\n  title My day\n  section Go\n  Make tea: 5: Me",
            "mindmap\n  root((mm))\n    A\n    B",
            "gitGraph\n  commit\n  branch dev\n  commit",
            "timeline\n  title T\n  2002 : LinkedIn",
        ];
        for source in sources {
            let image = render(&DiagramKey::new(source, &style()), 0, 1.0);
            assert!(image.is_ok(), "{source:?}: {:?}", image.err());
        }
    }

    #[test]
    fn invalid_empty_and_oversized_sources_fail_with_one_line() {
        for source in ["flowchart LR\n  A -->\n", "", "   \n", "not a diagram"] {
            let error = render(&DiagramKey::new(source, &style()), 0, 1.0)
                .err()
                .unwrap();
            assert!(!error.is_empty() && !error.contains('\n'), "{error:?}");
        }
        let huge = format!("flowchart TD\n{}", "  A --> B\n".repeat(8000));
        assert!(
            render(&DiagramKey::new(&huge, &style()), 0, 1.0)
                .err()
                .unwrap()
                .contains("64 KB")
        );
    }

    #[test]
    fn raster_sizes_respect_the_width_and_pixel_limits() {
        assert_eq!(pixel_size((100, 50), 0, 1.5), Some((150, 75)));
        assert_eq!(pixel_size((100, 50), 120, 2.0), Some((120, 60)));
        // Below one pixel per DIP only when the pane is narrower than the diagram.
        assert_eq!(pixel_size((100, 50), 0, 0.5), Some((100, 50)));
        assert_eq!(pixel_size((100, 50), 0, f32::NAN), Some((100, 50)));
        let (width, height) = pixel_size((6000, 6000), 0, 2.0).unwrap();
        assert!(u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS);
        assert_eq!(pixel_size((10_000, 10_000), 0, 1.0), None);
    }

    #[test]
    fn neighbouring_pie_slices_differ_clearly_on_light_and_dark_backgrounds() {
        for dark in [false, true] {
            let background = colors(dark).background;
            for index in 0..11 {
                let (a, b) = (
                    pie_color(background, index),
                    pie_color(background, index + 1),
                );
                let distance = [0, 8, 16]
                    .iter()
                    .map(|shift| ((a >> shift) & 0xFF).abs_diff((b >> shift) & 0xFF))
                    .sum::<u32>();
                assert!(
                    distance > 60,
                    "slices {index} and {}: {a:06X} {b:06X}",
                    index + 1
                );
            }
        }
    }

    #[test]
    fn keys_differ_by_source_and_style() {
        let a = DiagramKey::new("graph LR\n A-->B", &style());
        assert_eq!(a, DiagramKey::new("graph LR\n A-->B", &style()));
        assert_ne!(a, DiagramKey::new("graph LR\n A-->C", &style()));
        let dark = DiagramStyle::new(colors(true), "Segoe UI", 16.0);
        assert_ne!(a, DiagramKey::new("graph LR\n A-->B", &dark));
    }
}
