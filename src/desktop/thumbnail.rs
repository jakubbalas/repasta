//! Draws an image clip's small RGBA thumbnail with quads, so the panel needs
//! no image decoder or image widget. Runs of similar pixels in a row become
//! one quad.

use std::sync::Arc;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Tree, Widget};
use iced::{Border, Color, Element, Length, Rectangle, Size, mouse};
use repasta::Thumbnail;

/// Largest per-channel difference merged into one run.
const RUN_TOLERANCE: u8 = 6;
/// Overlap between neighbouring quads, hiding anti-aliasing seams.
const OVERLAP: f32 = 0.35;

pub struct Preview {
    thumbnail: Option<Arc<Thumbnail>>,
    size: Size,
    backdrop: Color,
    outline: Color,
}

impl Preview {
    pub fn new(
        thumbnail: Option<Arc<Thumbnail>>,
        size: Size,
        backdrop: Color,
        outline: Color,
    ) -> Self {
        Self {
            thumbnail,
            size,
            backdrop,
            outline,
        }
    }
}

impl<Message, Theme, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer> for Preview {
    fn size(&self) -> Size<Length> {
        Size::new(
            Length::Fixed(self.size.width),
            Length::Fixed(self.size.height),
        )
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.resolve(
            Length::Fixed(self.size.width),
            Length::Fixed(self.size.height),
            self.size,
        ))
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        if !bounds.intersects(viewport) {
            return;
        }
        renderer.fill_quad(
            Quad {
                bounds,
                border: Border {
                    color: self.outline,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..Quad::default()
            },
            self.backdrop,
        );
        let Some(thumbnail) = self.thumbnail.as_deref() else {
            return;
        };
        let (columns, rows) = (usize::from(thumbnail.width), usize::from(thumbnail.height));
        if columns == 0 || rows == 0 || thumbnail.rgba.len() != columns * rows * 4 {
            return;
        }

        // Fit inside the border, keeping the aspect ratio.
        let inner = bounds.shrink(2.0);
        let cell = (inner.width / columns as f32).min(inner.height / rows as f32);
        let origin_x = inner.x + (inner.width - cell * columns as f32) / 2.0;
        let origin_y = inner.y + (inner.height - cell * rows as f32) / 2.0;

        for (row, pixels) in thumbnail.rgba.chunks_exact(columns * 4).enumerate() {
            let y = origin_y + row as f32 * cell;
            let mut start = 0;
            while start < columns {
                let first = &pixels[start * 4..start * 4 + 4];
                let mut end = start + 1;
                while end < columns
                    && pixels[end * 4..end * 4 + 4]
                        .iter()
                        .zip(first)
                        .all(|(a, b)| a.abs_diff(*b) <= RUN_TOLERANCE)
                {
                    end += 1;
                }
                if first[3] > 0 {
                    renderer.fill_quad(
                        Quad {
                            bounds: Rectangle {
                                x: origin_x + start as f32 * cell,
                                y,
                                width: (end - start) as f32 * cell + OVERLAP,
                                height: cell + OVERLAP,
                            },
                            ..Quad::default()
                        },
                        Color::from_rgba8(
                            first[0],
                            first[1],
                            first[2],
                            f32::from(first[3]) / 255.0,
                        ),
                    );
                }
                start = end;
            }
        }
    }
}

impl<'a, Message, Theme, Renderer> From<Preview> for Element<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer + 'a,
{
    fn from(preview: Preview) -> Self {
        Element::new(preview)
    }
}
