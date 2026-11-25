//! A column whose rows slide to their new places, which the Agents section and Tasks
//! mode use so an Agent whose state moves it is seen going there (ADR 0075).

use gpui_kit::*;
use std::collections::HashMap;
use std::time::Instant;

/// The Agents section's fold spring: critically damped, so a row never passes its place.
const SLIDE_SPRING: SpringConfig = SpringConfig::new(900., 60., 1.);
/// Settled within half a pixel.
const SLIDE_EPSILON: f32 = 0.5;

/// A vertical stack of rows, each under a key of its own. A row laid out somewhere new in
/// the column is drawn where it was and slides to its place; one that comes or goes does
/// so at once, and the rows after it slide. Places are taken from the column's top, so
/// scrolling or resizing what holds it moves nothing.
pub(in crate::app) struct SlideColumn {
    id: ElementId,
    style: StyleRefinement,
    rows: Vec<(ElementId, AnyElement)>,
}

impl SlideColumn {
    pub(in crate::app) fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            rows: Vec::new(),
        }
    }

    pub(in crate::app) fn rows(
        mut self,
        rows: impl IntoIterator<Item = (ElementId, AnyElement)>,
    ) -> Self {
        self.rows.extend(rows);
        self
    }
}

/// Where a row was laid out last, from the column's top, and how far from there it is
/// drawn.
struct Track {
    top: Pixels,
    offset: SpringState,
}

struct SlideState {
    rows: HashMap<ElementId, Track>,
    drawn_at: Instant,
}

impl Styled for SlideColumn {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for SlideColumn {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SlideColumn {
    type RequestLayoutState = Vec<LayoutId>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let rows = self
            .rows
            .iter_mut()
            .map(|(_, row)| row.request_layout(window, cx))
            .collect::<Vec<_>>();
        let mut style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            ..Style::default()
        };
        style.refine(&self.style);
        (window.request_layout(style, rows.iter().copied(), cx), rows)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layouts: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let now = cx.background_executor().now();
        let still = cx.reduce_motion();
        let id = id.expect("a SlideColumn has an id");
        let moving = window.with_element_state(id, |state: Option<SlideState>, window| {
            let (mut last, elapsed) = match state {
                Some(state) => (state.rows, (now - state.drawn_at).as_secs_f32()),
                None => (HashMap::new(), 0.),
            };
            let mut rows = HashMap::with_capacity(self.rows.len());
            let mut moving = false;
            for ((key, row), layout) in self.rows.iter_mut().zip(layouts.iter()) {
                let top = window.layout_bounds(*layout).top() - bounds.top();
                let mut offset = SpringState::default();
                if let Some(track) = last.remove(key) {
                    // Where the slide it was on has brought it, then back over its move.
                    offset = SLIDE_SPRING.step(track.offset, 0., elapsed);
                    offset.position += f32::from(track.top - top);
                }
                if still || SLIDE_SPRING.is_settled(offset, 0., SLIDE_EPSILON) {
                    offset = SpringState::default();
                } else {
                    moving = true;
                }
                window.with_element_offset(point(px(0.), px(offset.position)), |window| {
                    row.prepaint(window, cx)
                });
                rows.insert(key.clone(), Track { top, offset });
            }
            (
                moving,
                SlideState {
                    rows,
                    drawn_at: now,
                },
            )
        });
        if moving {
            window.request_animation_frame();
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        for (_, row) in &mut self.rows {
            row.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's own `test` attribute would shadow the standard one.
    use super::SlideColumn;
    use gpui_kit::{
        Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
        Render, Styled as _, TestAppContext, VisualTestContext, Window, div, px,
    };
    use std::time::Duration;

    struct Rows(Vec<&'static str>);

    impl Render for Rows {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child(SlideColumn::new("rows").rows(self.0.iter().map(|name| {
                let name = *name;
                let row = div().h(px(20.)).debug_selector(move || name.into());
                (ElementId::from(name), row.into_any_element())
            })))
        }
    }

    #[test]
    fn a_row_that_moves_slides_from_where_it_was() {
        let mut cx = TestAppContext::single();
        let (view, window) = cx.add_window_view(|_, _| Rows(vec!["a", "b"]));
        let draw_after = |window: &mut VisualTestContext, millis| {
            window
                .executor()
                .advance_clock(Duration::from_millis(millis));
            window.update(|window, cx| _ = window.draw(cx));
        };
        let tops = |window: &mut VisualTestContext| -> (Pixels, Pixels) {
            let mut top = |name| window.debug_bounds(name).unwrap().top();
            (top("a"), top("b"))
        };
        let swap = |window: &mut VisualTestContext| {
            window.update(|_, cx| {
                view.update(cx, |rows, cx| {
                    rows.0.reverse();
                    cx.notify();
                })
            });
        };
        draw_after(window, 0);
        let (a, b) = tops(window);

        swap(window);
        draw_after(window, 0);
        assert_eq!(tops(window), (a, b), "each row is drawn where it was");
        draw_after(window, 60);
        let (sliding, _) = tops(window);
        assert!(a < sliding && sliding < b, "then slides");
        draw_after(window, 500);
        assert_eq!(tops(window), (b, a), "and settles in its place");

        window.update(|_, cx| cx.set_reduce_motion(true));
        swap(window);
        draw_after(window, 0);
        assert_eq!(
            tops(window),
            (a, b),
            "with reduced motion it is there at once"
        );
    }
}
