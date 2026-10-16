//! Book pages: lays a chapter out as one tall column, then shows it a page
//! at a time (or two side by side) through clipped windows onto that
//! column, like a Kindle.
//!
//! Page breaks always fall between lines. The caller tags each child of the
//! column with a [`BoxKind`]: text blocks are split at their line height,
//! images and rules move to the next page whole, and gaps between blocks
//! are dropped at the top of a page. Each window is clipped to the lines
//! that fit, so no line is ever cut in half.
//!
//! The breaks are worked out during layout, when the size is known, and
//! shared with the app through [`Pages`] so page turns know where the next
//! page starts.

use std::sync::{Arc, Mutex};

use cosmic::iced::advanced::layout::{self, Layout};
use cosmic::iced::advanced::widget::{tree, Tree};
use cosmic::iced::advanced::{mouse, renderer, Clipboard, Shell, Widget};
use cosmic::iced::mouse::ScrollDelta;
use cosmic::iced::{touch, Element, Event, Length, Point, Rectangle, Size, Vector};

/// How a child of the paged column may be split across pages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxKind {
    /// Spacing between blocks; skipped at the top of a page.
    Gap,
    /// Text with this line height; may break between any two lines.
    Lines(f32),
    /// Kept whole (rules, scene breaks, "The End").
    Solid,
    /// Kept whole like `Solid`; clicking it reports this image index.
    Image(usize),
}

/// Where the current chapter's pages start and end, in the column's
/// coordinates. Written by the widget on every layout.
#[derive(Clone, Debug, Default)]
pub struct Pagination {
    pub chapter: usize,
    pub starts: Vec<f32>,
    pub ends: Vec<f32>,
    pub content_h: f32,
    /// Height of one page window.
    pub page_h: f32,
    /// Two pages are on screen at once.
    pub spread: bool,
}

pub type Pages = Arc<Mutex<Pagination>>;

impl Pagination {
    pub fn count(&self) -> usize {
        self.starts.len()
    }

    /// Pages turned at once.
    pub fn step(&self) -> usize {
        if self.spread { 2 } else { 1 }
    }

    /// The (first) page on screen for a position, given as a fraction of
    /// the chapter's height. 1.0 lands on the last page.
    pub fn page_at(&self, anchor: f32) -> usize {
        let y = anchor.clamp(0.0, 1.0) * self.content_h;
        let i = self.starts.iter().rposition(|s| *s <= y + 0.5).unwrap_or(0);
        if self.spread { i - i % 2 } else { i }
    }

    /// The position of a page's top, as a fraction of the chapter.
    pub fn anchor_of(&self, page: usize) -> f32 {
        match self.starts.get(page) {
            Some(s) if self.content_h > 0.0 => s / self.content_h,
            _ => 0.0,
        }
    }

    /// How far through the chapter the reader is once the pages on screen
    /// are read.
    pub fn read_through(&self, anchor: f32) -> f32 {
        let last = (self.page_at(anchor) + self.step() - 1).min(self.count().saturating_sub(1));
        match self.ends.get(last) {
            Some(e) if self.content_h > 0.0 => (e / self.content_h).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    fn breaks(&mut self, boxes: &[(f32, f32)]) {
        self.starts = vec![0.0];
        self.ends.clear();
        let mut start = 0.0;
        let mut last_bottom = 0.0f32;
        for &(top, bottom) in boxes {
            if bottom - start > self.page_h + 0.5 {
                if last_bottom > start {
                    self.ends.push(last_bottom);
                    self.starts.push(top);
                } else if let Some(first) = self.starts.last_mut() {
                    // Nothing but spacing on this page yet: drop the
                    // spacing rather than cut the block off.
                    *first = top;
                }
                start = top;
            }
            last_bottom = last_bottom.max(bottom);
        }
        self.ends.push(last_bottom.max(start));
    }
}

#[derive(Default)]
struct State {
    /// Left edge of each page window, relative to the widget.
    xs: [f32; 2],
    col_w: f32,
    /// The pagination changed since the app last heard about it.
    dirty: bool,
    /// Unspent trackpad scroll, in pixels.
    wheel: f32,
    /// Where each image sits in the column (column coordinates).
    images: Vec<(usize, Rectangle)>,
    /// Page windows on screen, relative to the widget, with the column y
    /// each one starts at.
    windows: Vec<(Rectangle, f32)>,
}

impl State {
    /// The image under a point (relative to the widget), if any.
    fn image_at(&self, p: Point) -> Option<usize> {
        let (win, start) = self.windows.iter().find(|(w, _)| w.contains(p))?;
        let at = Point::new(p.x - win.x, p.y - win.y + start);
        self.images.iter().find(|(_, r)| r.contains(at)).map(|(i, _)| *i)
    }
}

pub struct Paged<'a, Message> {
    content: Element<'a, Message, cosmic::Theme, cosmic::Renderer>,
    kinds: Vec<BoxKind>,
    pages: Pages,
    chapter: usize,
    anchor: f32,
    two_pages: bool,
    measure: f32,
    min_col: f32,
    gutter: f32,
    pad: f32,
    on_next: Message,
    on_prev: Message,
    on_repaginate: Message,
    on_image: Option<Box<dyn Fn(usize) -> Message + 'a>>,
}

impl<'a, Message: Clone> Paged<'a, Message> {
    /// `content` is a column whose children are described, in order, by
    /// `kinds`. `anchor` is the position to show (see
    /// [`Pagination::page_at`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        content: impl Into<Element<'a, Message, cosmic::Theme, cosmic::Renderer>>,
        kinds: Vec<BoxKind>,
        pages: Pages,
        chapter: usize,
        anchor: f32,
        on_next: Message,
        on_prev: Message,
        on_repaginate: Message,
    ) -> Self {
        Self {
            content: content.into(),
            kinds,
            pages,
            chapter,
            anchor,
            two_pages: false,
            measure: f32::INFINITY,
            min_col: 0.0,
            gutter: 24.0,
            pad: 24.0,
            on_next,
            on_prev,
            on_repaginate,
            on_image: None,
        }
    }

    /// Clicking an image (away from the page-turn edges) sends this.
    pub fn on_image(mut self, f: impl Fn(usize) -> Message + 'a) -> Self {
        self.on_image = Some(Box::new(f));
        self
    }

    /// Show two pages side by side when each still gets `min_col` width.
    pub fn two_pages(mut self, on: bool, min_col: f32) -> Self {
        self.two_pages = on;
        self.min_col = min_col;
        self
    }

    /// Widest a page's text column may get.
    pub fn measure(mut self, w: f32) -> Self {
        self.measure = w;
        self
    }

    /// Margin around the pages (and between two of them).
    pub fn margins(mut self, side: f32, top: f32) -> Self {
        self.gutter = side;
        self.pad = top;
        self
    }
}

impl<Message: Clone> Widget<Message, cosmic::Theme, cosmic::Renderer> for Paged<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &cosmic::Renderer, limits: &layout::Limits) -> layout::Node {
        let size = limits.width(Length::Fill).height(Length::Fill).max();
        let avail = (size.width - 2.0 * self.gutter).max(1.0);
        let half = (avail - self.gutter) / 2.0;
        let spread = self.two_pages && half >= self.min_col;
        let col_w = if spread { half.min(self.measure) } else { avail.min(self.measure) };
        let xs = if spread {
            let left = (size.width - (2.0 * col_w + self.gutter)) / 2.0;
            [left, left + col_w + self.gutter]
        } else {
            let left = (size.width - col_w) / 2.0;
            [left, left]
        };

        let child_limits = layout::Limits::new(Size::ZERO, Size::new(col_w, f32::INFINITY));
        let child = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, &child_limits);

        // Every line (or unbreakable block) as a (top, bottom) span.
        let mut boxes = Vec::new();
        let mut images = Vec::new();
        for (i, node) in child.children().iter().enumerate() {
            let b = node.bounds();
            match self.kinds.get(i).copied().unwrap_or(BoxKind::Solid) {
                BoxKind::Gap => {}
                BoxKind::Solid => boxes.push((b.y, b.y + b.height)),
                BoxKind::Image(idx) => {
                    boxes.push((b.y, b.y + b.height));
                    // The picture itself, inside its centering container.
                    let pic = node.children().first().map_or(b, |n| {
                        let r = n.bounds();
                        Rectangle { x: b.x + r.x, y: b.y + r.y, ..r }
                    });
                    images.push((idx, pic));
                }
                BoxKind::Lines(lh) => {
                    let n = (b.height / lh.max(1.0)).round().max(1.0) as usize;
                    for j in 0..n {
                        let top = b.y + b.height * j as f32 / n as f32;
                        let bottom = b.y + b.height * (j + 1) as f32 / n as f32;
                        boxes.push((top, bottom));
                    }
                }
            }
        }

        let mut p = Pagination {
            chapter: self.chapter,
            content_h: child.size().height.max(1.0),
            page_h: (size.height - 2.0 * self.pad).max(1.0),
            spread,
            ..Pagination::default()
        };
        p.breaks(&boxes);

        let state = tree.state.downcast_mut::<State>();
        state.xs = xs;
        state.col_w = col_w;
        state.images = images;
        if let Ok(mut shared) = self.pages.lock() {
            let changed = shared.chapter != p.chapter
                || shared.starts != p.starts
                || shared.spread != p.spread
                || (shared.page_h - p.page_h).abs() > 0.5;
            if changed {
                *shared = p;
                state.dirty = true;
            }
        }

        layout::Node::with_children(size, vec![child])
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &cosmic::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if state.dirty {
            // Let the app redraw its page counter for the new layout.
            state.dirty = false;
            shell.publish(self.on_repaginate.clone());
        }

        let bounds = layout.bounds();
        if let Ok(pages) = self.pages.lock() {
            let first = pages.page_at(self.anchor);
            state.windows = (0..pages.step())
                .filter_map(|c| {
                    let start = *pages.starts.get(first + c)?;
                    let end = *pages.ends.get(first + c)?;
                    let win = Rectangle {
                        x: state.xs[c],
                        y: self.pad,
                        width: state.col_w,
                        height: (end - start).min(pages.page_h),
                    };
                    Some((win, start))
                })
                .collect();
        }
        // Click or tap the left or right edge to turn the page.
        let edge = |p: Point| {
            let rel = (p.x - bounds.x) / bounds.width.max(1.0);
            if rel < 0.3 {
                Some(false)
            } else if rel > 0.7 {
                Some(true)
            } else {
                None
            }
        };
        let turn = |forward: bool, shell: &mut Shell<'_, Message>| {
            shell.publish(if forward { self.on_next.clone() } else { self.on_prev.clone() });
            shell.capture_event();
        };

        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => match *delta {
                ScrollDelta::Lines { y, .. } if y != 0.0 => turn(y < 0.0, shell),
                ScrollDelta::Pixels { y, .. } => {
                    // Trackpads send a stream of small deltas; one page
                    // per decent swipe.
                    state.wheel += y;
                    if state.wheel.abs() >= 80.0 {
                        let forward = state.wheel < 0.0;
                        state.wheel = 0.0;
                        turn(forward, shell);
                    } else {
                        shell.capture_event();
                    }
                }
                _ => {}
            },
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(p) = cursor.position_over(bounds) {
                    let img = state.image_at(Point::new(p.x - bounds.x, p.y - bounds.y));
                    self.press(edge(p), img, shell);
                }
            }
            Event::Touch(touch::Event::FingerPressed { position, .. }) if bounds.contains(*position) => {
                let img = state.image_at(Point::new(position.x - bounds.x, position.y - bounds.y));
                self.press(edge(*position), img, shell);
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &cosmic::Renderer,
    ) -> mouse::Interaction {
        let bounds = layout.bounds();
        let state = tree.state.downcast_ref::<State>();
        match cursor.position_over(bounds) {
            Some(p) => {
                let rel = (p.x - bounds.x) / bounds.width.max(1.0);
                let middle = (0.3..=0.7).contains(&rel);
                if middle && self.on_image.is_some() && state.image_at(Point::new(p.x - bounds.x, p.y - bounds.y)).is_some() {
                    mouse::Interaction::ZoomIn
                } else if !middle {
                    mouse::Interaction::Pointer
                } else {
                    mouse::Interaction::Idle
                }
            }
            None => mouse::Interaction::Idle,
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut cosmic::Renderer,
        theme: &cosmic::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use cosmic::iced::advanced::Renderer as _;

        let state = tree.state.downcast_ref::<State>();
        let Ok(pages) = self.pages.lock() else { return };
        if pages.chapter != self.chapter || pages.count() == 0 {
            return;
        }
        let Some(child) = layout.children().next() else { return };
        let bounds = layout.bounds();
        let first = pages.page_at(self.anchor);

        for (col, x) in state.xs.iter().enumerate().take(pages.step()) {
            let page = first + col;
            let (Some(&start), Some(&end)) = (pages.starts.get(page), pages.ends.get(page)) else { break };
            let clip = Rectangle {
                x: bounds.x + x,
                y: bounds.y + self.pad,
                width: state.col_w,
                height: (end - start).min(pages.page_h),
            };
            let shift = Vector::new(*x, self.pad - start);
            // The part of the column this window shows, in the column's
            // own (untranslated) coordinates.
            let visible = Rectangle { x: clip.x - shift.x, y: clip.y - shift.y, ..clip };
            renderer.with_layer(clip, |r| {
                r.with_translation(shift, |r| {
                    self.content.as_widget().draw(
                        &tree.children[0],
                        r,
                        theme,
                        style,
                        child,
                        mouse::Cursor::Unavailable,
                        &visible,
                    );
                });
            });
        }
    }
}

impl<Message: Clone> Paged<'_, Message> {
    /// A click or tap: the edges turn pages, an image in between opens it.
    fn press(
        &self,
        edge: Option<bool>,
        image: Option<usize>,
        shell: &mut Shell<'_, Message>,
    ) {
        if let Some(forward) = edge {
            shell.publish(if forward { self.on_next.clone() } else { self.on_prev.clone() });
            shell.capture_event();
        } else if let (Some(f), Some(idx)) = (&self.on_image, image) {
            shell.publish(f(idx));
            shell.capture_event();
        }
    }
}

impl<'a, Message: Clone + 'a> From<Paged<'a, Message>> for Element<'a, Message, cosmic::Theme, cosmic::Renderer> {
    fn from(p: Paged<'a, Message>) -> Self {
        Element::new(p)
    }
}
