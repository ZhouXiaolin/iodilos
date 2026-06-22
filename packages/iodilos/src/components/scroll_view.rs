use std::ops::Range;
use std::rc::Rc;

use crate::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrollWindow {
    pub range: Range<usize>,
    pub total: usize,
}

pub type ScrollContent = Rc<dyn Fn(&ScrollWindow) -> View>;

#[derive(Clone)]
pub struct ScrollViewProps {
    pub total: ReadSignal<usize>,
    pub anchor: ReadSignal<usize>,
    pub max_visible: usize,
    pub content: ScrollContent,
}

pub fn scroll_view(props: ScrollViewProps) -> View {
    View::from_dynamic(move || {
        let total = props.total.get();
        let anchor = props.anchor.get().min(total.saturating_sub(1));
        let max_visible = props.max_visible.max(1);
        let (start, end) = centered_window(total, anchor, max_visible);
        let window = ScrollWindow {
            range: start..end,
            total,
        };

        View::from(
            tags::div()
                .flex_direction(FlexDirection::Column)
                .width(Size::Percent(100.0))
                .overflow(Overflow::Hidden)
                .children((props.content)(&window)),
        )
    })
}

pub fn centered_window(total: usize, anchor: usize, max_visible: usize) -> (usize, usize) {
    if total <= max_visible {
        return (0, total);
    }
    let half = max_visible / 2;
    let mut start = anchor.saturating_sub(half);
    start = start.min(total - max_visible);
    (start, start + max_visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_window_keeps_anchor_near_middle() {
        assert_eq!(centered_window(20, 10, 5), (8, 13));
        assert_eq!(centered_window(20, 1, 5), (0, 5));
        assert_eq!(centered_window(20, 19, 5), (15, 20));
    }
}
