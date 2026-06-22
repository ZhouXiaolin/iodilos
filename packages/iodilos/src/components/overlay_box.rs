use crate::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayGeometry {
    FullBleed,
    Inset { ratio: f32 },
}

pub struct OverlayBoxProps {
    pub geometry: OverlayGeometry,
    pub background: Color,
    pub border_style: BorderStyle,
    pub border_color: Color,
    pub content: View,
}

pub fn overlay_box(props: OverlayBoxProps) -> View {
    let inset = match props.geometry {
        OverlayGeometry::FullBleed => Inset::Length(0),
        OverlayGeometry::Inset { ratio } => Inset::Percent((ratio * 100.0).clamp(0.0, 49.0)),
    };

    View::from(
        tags::div()
            .position(Position::Absolute)
            .top(inset)
            .right(inset)
            .bottom(inset)
            .left(inset)
            .background_color(props.background)
            .border_style(props.border_style)
            .border_color(props.border_color)
            .children(props.content),
    )
}
