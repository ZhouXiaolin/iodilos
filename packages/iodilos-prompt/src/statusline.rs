//! Statusline model + Solaren's mock values.

use std::borrow::Cow;

use iodilos::Color;

/// One `icon text` field of the statusline, joined to its neighbours by ` > `.
#[derive(Debug, Clone)]
pub struct StatusField {
    pub icon: Cow<'static, str>,
    pub text: Cow<'static, str>,
    pub color: Color,
}

/// The statusline payload rendered on the prompt's top border:
/// `brand > icon text > icon text > … tail`.
#[derive(Debug, Clone)]
pub struct StatusLine {
    pub brand: Cow<'static, str>,
    pub brand_color: Color,
    pub fields: Vec<StatusField>,
    pub tail: Cow<'static, str>,
    pub tail_color: Color,
}

impl StatusLine {
    /// Solaren's mock values: model + reasoning level, cwd, branch, context usage.
    /// Real apps populate these from env / git / a token counter.
    pub fn default_mock() -> Self {
        use Cow::Borrowed;
        Self {
            brand: Borrowed("π"),
            brand_color: Color::Magenta,
            fields: vec![
                StatusField {
                    icon: Borrowed("⬢"),
                    text: Borrowed("MiMo-V2.5-Pro++ · ◕ high"),
                    color: Color::Cyan,
                },
                StatusField {
                    icon: Borrowed("📁"),
                    text: Borrowed("~/iodilos"),
                    color: Color::Blue,
                },
                StatusField {
                    icon: Borrowed("⑂"),
                    text: Borrowed("master"),
                    color: Color::Green,
                },
                StatusField {
                    icon: Borrowed("◫"),
                    text: Borrowed("2.1%/1M ⟲"),
                    color: Color::Yellow,
                },
            ],
            tail: Borrowed("▶"),
            tail_color: Color::DarkGrey,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mock_matches_spec() {
        let sl = StatusLine::default_mock();
        assert_eq!(sl.brand.as_ref(), "π");
        assert_eq!(sl.tail.as_ref(), "▶");
        assert_eq!(sl.fields.len(), 4);
        assert_eq!(sl.fields[0].icon.as_ref(), "⬢");
        assert_eq!(sl.fields[0].text.as_ref(), "MiMo-V2.5-Pro++ · ◕ high");
        assert_eq!(sl.fields[2].icon.as_ref(), "⑂");
        assert_eq!(sl.fields[2].text.as_ref(), "master");
        assert_eq!(sl.fields[3].text.as_ref(), "2.1%/1M ⟲");
    }
}
