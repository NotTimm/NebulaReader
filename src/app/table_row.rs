//! One row of a table: its cells side by side, each given a share of the
//! width, and all as tall as the tallest, so their borders make a grid.
//!
//! An iced row can't do the second part on a page that scrolls: a cell set
//! to fill the row's height is laid out against a height of 0 there. This
//! lays every cell out at its own height first, then again at the
//! tallest.

use cosmic::iced::advanced::layout::{self, Layout};
use cosmic::iced::advanced::widget::{Operation, Tree};
use cosmic::iced::advanced::{mouse, overlay, renderer, Clipboard, Shell, Widget};
use cosmic::iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

pub struct TableRow<'a, Message> {
    children: Vec<Element<'a, Message, cosmic::Theme, cosmic::Renderer>>,
    /// Each cell's share of the width.
    weights: Vec<f32>,
    spacing: f32,
}

impl<'a, Message> TableRow<'a, Message> {
    pub fn new() -> Self {
        Self { children: Vec::new(), weights: Vec::new(), spacing: 0.0 }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// A cell with its share of the width.
    pub fn push(mut self, weight: f32, child: impl Into<Element<'a, Message, cosmic::Theme, cosmic::Renderer>>) -> Self {
        self.weights.push(weight.max(0.1));
        self.children.push(child.into());
        self
    }
}

/// The cells' widths: the row's width less the spacing, shared by weight.
pub fn widths(total: f32, weights: &[f32], spacing: f32) -> Vec<f32> {
    let sum: f32 = weights.iter().sum();
    let free = (total - spacing * weights.len().saturating_sub(1) as f32).max(0.0);
    weights.iter().map(|w| if sum > 0.0 { free * w / sum } else { 0.0 }).collect()
}

impl<Message> Widget<Message, cosmic::Theme, cosmic::Renderer> for TableRow<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut self.children);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &cosmic::Renderer, limits: &layout::Limits) -> layout::Node {
        let width = limits.width(Length::Fill).max().width;
        let widths = widths(width, &self.weights, self.spacing);
        // Each cell at its own height, to find the tallest.
        let mut height: f32 = 0.0;
        for ((child, state), w) in self.children.iter_mut().zip(&mut tree.children).zip(&widths) {
            let node = child.as_widget_mut().layout(state, renderer, &layout::Limits::new(Size::new(*w, 0.0), Size::new(*w, f32::INFINITY)));
            height = height.max(node.size().height);
        }
        // Then all at that height.
        let mut x = 0.0;
        let mut nodes = Vec::with_capacity(self.children.len());
        for ((child, state), w) in self.children.iter_mut().zip(&mut tree.children).zip(&widths) {
            let node = child.as_widget_mut().layout(state, renderer, &layout::Limits::new(Size::new(*w, height), Size::new(*w, height)));
            nodes.push(node.move_to(Point::new(x, 0.0)));
            x += w + self.spacing;
        }
        layout::Node::with_children(Size::new(width, height), nodes)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &cosmic::Renderer, operation: &mut dyn Operation) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            self.children.iter_mut().zip(&mut tree.children).zip(layout.children()).for_each(|((child, state), c_layout)| {
                child.as_widget_mut().operate(state, c_layout.with_virtual_offset(layout.virtual_offset()), renderer, operation);
            });
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &cosmic::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        for ((child, state), c_layout) in self.children.iter_mut().zip(&mut tree.children).zip(layout.children()) {
            child.as_widget_mut().update(state, event, c_layout.with_virtual_offset(layout.virtual_offset()), cursor, renderer, clipboard, shell, viewport);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &cosmic::Renderer,
    ) -> mouse::Interaction {
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, state), c_layout)| {
                child.as_widget().mouse_interaction(state, c_layout.with_virtual_offset(layout.virtual_offset()), cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut cosmic::Renderer,
        theme: &cosmic::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((child, state), c_layout) in self.children.iter().zip(&tree.children).zip(layout.children()) {
            child.as_widget().draw(state, renderer, theme, style, c_layout.with_virtual_offset(layout.virtual_offset()), cursor, viewport);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &cosmic::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, cosmic::Theme, cosmic::Renderer>> {
        overlay::from_children(&mut self.children, tree, layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<TableRow<'a, Message>> for Element<'a, Message, cosmic::Theme, cosmic::Renderer> {
    fn from(r: TableRow<'a, Message>) -> Self {
        Element::new(r)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cells_share_the_width_by_weight() {
        let w = super::widths(100.0, &[1.0, 3.0], 0.0);
        assert_eq!(w, [25.0, 75.0]);
        let w = super::widths(110.0, &[1.0, 1.0, 2.0], 5.0);
        assert_eq!(w, [25.0, 25.0, 50.0]);
    }
}
