//! A column whose items can float right, like CSS `float: right` on
//! Wikipedia's pages: a floating item (an infobox, a picture) sits at the
//! right edge, the items after it are laid out beside it, narrower, and
//! once it ends they take the whole width again. A float below another one
//! starts under it, as with `clear: right`.
//!
//! An item beside a float is laid out at the narrower width whole, so a
//! paragraph that runs past the float's end stays narrow to its last line
//! (a browser would widen it mid-paragraph); between paragraphs the text
//! goes back to full width.

use cosmic::iced::advanced::layout::{self, Layout};
use cosmic::iced::advanced::widget::{Operation, Tree};
use cosmic::iced::advanced::{mouse, overlay, renderer, Clipboard, Shell, Widget};
use cosmic::iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

pub struct FloatFlow<'a, Message> {
    children: Vec<Element<'a, Message, cosmic::Theme, cosmic::Renderer>>,
    floats: Vec<bool>,
    spacing: f32,
    /// Space between a float and the text beside it.
    gap: f32,
}

impl<'a, Message> FloatFlow<'a, Message> {
    pub fn new() -> Self {
        Self { children: Vec::new(), floats: Vec::new(), spacing: 0.0, gap: 0.0 }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// An item in the flow, full width unless beside a float.
    pub fn push(mut self, child: impl Into<Element<'a, Message, cosmic::Theme, cosmic::Renderer>>) -> Self {
        self.children.push(child.into());
        self.floats.push(false);
        self
    }

    /// An item floated to the right edge, at its own width.
    pub fn float(mut self, child: impl Into<Element<'a, Message, cosmic::Theme, cosmic::Renderer>>) -> Self {
        self.children.push(child.into());
        self.floats.push(true);
        self
    }
}

impl<'a, Message> Default for FloatFlow<'a, Message> {
    fn default() -> Self {
        Self::new()
    }
}

/// Where each item goes: its top-left and width, for a flow `width` wide
/// with items of the given heights (`height(i, width)`).
pub fn place(
    width: f32,
    floats: &[bool],
    spacing: f32,
    gap: f32,
    mut measure: impl FnMut(usize, f32) -> Size,
) -> (Vec<(Point, Size)>, f32) {
    let mut out = Vec::with_capacity(floats.len());
    let mut y = 0.0f32;
    // Floats placed so far: left edge, top, bottom.
    let mut placed: Vec<(f32, f32, f32)> = Vec::new();
    let mut first = true;
    for (i, &is_float) in floats.iter().enumerate() {
        if is_float {
            let size = measure(i, (width * 0.6).max(1.0));
            // Below the last float, and no higher than the text so far.
            let top = match placed.last() {
                Some(&(_, _, bottom)) => y.max(bottom + spacing),
                None => y,
            };
            let left = (width - size.width).max(0.0);
            out.push((Point::new(left, top), size));
            placed.push((left, top, top + size.height));
        } else {
            if !first {
                y += spacing;
            }
            first = false;
            // Narrower beside the float at this height, if there is one.
            let beside = placed.iter().filter(|&&(_, top, bottom)| top <= y && y < bottom).map(|&(left, _, _)| left).fold(f32::INFINITY, f32::min);
            let avail = if beside.is_finite() { (beside - gap).max(1.0) } else { width };
            let size = measure(i, avail);
            out.push((Point::new(0.0, y), Size::new(avail, size.height)));
            y += size.height;
        }
    }
    let float_bottom = placed.iter().map(|p| p.2).fold(0.0, f32::max);
    (out, y.max(float_bottom))
}

impl<Message> Widget<Message, cosmic::Theme, cosmic::Renderer> for FloatFlow<'_, Message> {
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
        let mut nodes: Vec<Option<layout::Node>> = (0..self.children.len()).map(|_| None).collect();
        let children = &mut self.children;
        let trees = &mut tree.children;
        let (spots, height) = place(width, &self.floats, self.spacing, self.gap, |i, avail| {
            let child_limits = layout::Limits::new(Size::ZERO, Size::new(avail, f32::INFINITY));
            let node = children[i].as_widget_mut().layout(&mut trees[i], renderer, &child_limits);
            let size = node.size();
            nodes[i] = Some(node);
            size
        });
        let nodes = nodes
            .into_iter()
            .zip(spots)
            .map(|(node, (at, _))| node.unwrap_or_default().move_to(at))
            .collect();
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
            child.as_widget_mut().update(
                state,
                event,
                c_layout.with_virtual_offset(layout.virtual_offset()),
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
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
        // Only what's on screen: a long article has hundreds of items.
        for ((child, state), c_layout) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .filter(|(_, l)| l.bounds().intersects(viewport))
        {
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

impl<'a, Message: 'a> From<FloatFlow<'a, Message>> for Element<'a, Message, cosmic::Theme, cosmic::Renderer> {
    fn from(f: FloatFlow<'a, Message>) -> Self {
        Element::new(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_wraps_beside_a_float_then_takes_the_width() {
        // A 300-wide, 250-tall infobox, then paragraphs 100 tall at any width.
        let floats = [true, false, false, false, false];
        let (spots, height) = place(1000.0, &floats, 10.0, 20.0, |i, _| if i == 0 { Size::new(300.0, 250.0) } else { Size::new(0.0, 100.0) });
        assert_eq!(spots[0].0, Point::new(700.0, 0.0));
        // Beside the infobox: narrower by its width and the gap.
        assert_eq!((spots[1].0.y, spots[1].1.width), (0.0, 680.0));
        assert_eq!((spots[2].0.y, spots[2].1.width), (110.0, 680.0));
        assert_eq!((spots[3].0.y, spots[3].1.width), (220.0, 680.0));
        // Below its end: the whole width again.
        assert_eq!((spots[4].0.y, spots[4].1.width), (330.0, 1000.0));
        assert_eq!(height, 430.0);
    }

    #[test]
    fn a_float_under_another_starts_below_it() {
        let floats = [true, false, true, false];
        let (spots, _) = place(1000.0, &floats, 10.0, 20.0, |i, _| match i {
            0 => Size::new(300.0, 400.0),
            2 => Size::new(200.0, 100.0),
            _ => Size::new(0.0, 50.0),
        });
        // The second float waits for the first to end.
        assert_eq!(spots[2].0, Point::new(800.0, 410.0));
        // The text after it is beside the first float's narrower column.
        assert_eq!(spots[3].1.width, 680.0);
    }
}
