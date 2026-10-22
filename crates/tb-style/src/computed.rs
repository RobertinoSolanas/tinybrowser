//! Computed style: the value objects the layout and paint phases consume.
//!
//! Relative lengths are normalised into a small linear form so that layout only
//! has to multiply by a basis:
//!
//! * [`L::Auto`]  — `auto`
//! * [`L::Px`]    — absolute pixels (viewport units are already folded in)
//! * [`L::Linear`] — `pct% of basis + fixed px` (this is what `calc()` with a
//!   single percentage reduces to)

use tb_core::Color;

/// Linear length: `pct/100 * basis + fixed`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum L {
    Auto,
    Px(f32),
    Linear { pct: f32, fixed: f32 },
}

impl L {
    pub fn resolve(&self, basis: f32) -> f32 {
        match self {
            // `auto` for width means "fill"; callers decide per property.
            L::Auto => basis,
            L::Px(v) => *v,
            L::Linear { pct, fixed } => basis * pct / 100.0 + fixed,
        }
    }

    /// Resolve for a property where `auto` means `0` (margin, padding, inset).
    pub fn resolve_zero(&self, basis: f32) -> f32 {
        match self {
            L::Auto => 0.0,
            L::Px(v) => *v,
            L::Linear { pct, fixed } => basis * pct / 100.0 + fixed,
        }
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, L::Auto)
    }

    pub fn px(v: f32) -> L {
        L::Px(v)
    }
}

/// `display` after mapping unknown values to something sane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    None,
    Inline,
    Block,
    InlineBlock,
    Flex,
    InlineFlex,
    ListItem,
    Table,
    TableRowGroup,
    TableRow,
    TableCell,
    /// `display: contents` — children are laid out as if the box were gone.
    Contents,
}

impl Display {
    pub fn is_inline(&self) -> bool {
        matches!(self, Display::Inline | Display::InlineBlock | Display::InlineFlex)
    }
    pub fn is_flex(&self) -> bool {
        matches!(self, Display::Flex | Display::InlineFlex)
    }
    pub fn is_tableish(&self) -> bool {
        matches!(
            self,
            Display::Table | Display::TableRow | Display::TableRowGroup | Display::TableCell
        )
    }
}

/// `position`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
    Scroll,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace {
    Normal,
    Nowrap,
    Pre,
    PreWrap,
    BreakSpaces,
}

impl WhiteSpace {
    pub fn preserves(&self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces)
    }
    pub fn wraps(&self) -> bool {
        matches!(
            self,
            WhiteSpace::Normal | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
}

/// Which border sides are painted (`none`/`hidden` are not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
}

impl BorderStyle {
    pub fn painted(&self) -> bool {
        *self != BorderStyle::None
    }
}

/// The computed style of one element: everything resolved for the *element*,
/// but relative lengths still need a basis from layout.
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    // box
    pub display: Display,
    pub position: Position,
    pub box_sizing_border: bool,
    pub width: L,
    pub height: L,
    pub min_width: L,
    pub max_width: L,
    pub min_height: L,
    pub max_height: L,
    pub margin: [L; 4],
    pub padding: [L; 4],
    pub border_width: [f32; 4],
    pub border_style: [BorderStyle; 4],
    pub border_color: [Color; 4],
    pub border_radius: [f32; 4],
    pub inset: [L; 4],
    pub z_index: Option<i32>,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub visibility: bool,
    pub opacity: f32,
    // text / colour
    pub color: Color,
    pub background_color: Color,
    pub background_image: bool,
    pub font_family: Vec<String>,
    pub font_size: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub line_height: f32,
    pub text_align: TextAlign,
    pub text_decoration: bool,
    pub text_decoration_line: bool,
    pub text_transform: TextTransform,
    pub white_space: WhiteSpace,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub vertical_align_top: bool,
    // flex
    pub flex_direction: FlexDirection,
    pub flex_wrap: bool,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_self: Option<AlignItems>,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: L,
    pub row_gap: f32,
    pub column_gap: f32,
    // list
    pub list_style_bullet: bool,
    pub list_style_number: bool,
    // misc that layout cares about
    pub float_left: bool,
    pub float_right: bool,
    pub clear: bool,
    /// `content: "text"` from `::before`/`::after`.
    pub before_content: Option<String>,
    pub after_content: Option<String>,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        ComputedStyle::initial()
    }
}

impl ComputedStyle {
    /// Initial (CSS) values — the root element starts here.
    pub fn initial() -> ComputedStyle {
        ComputedStyle {
            display: Display::Inline,
            position: Position::Static,
            box_sizing_border: false,
            width: L::Auto,
            height: L::Auto,
            min_width: L::Auto,
            max_width: L::Auto,
            min_height: L::Auto,
            max_height: L::Auto,
            margin: [L::Px(0.0); 4],
            padding: [L::Px(0.0); 4],
            border_width: [0.0; 4],
            border_style: [BorderStyle::None; 4],
            border_color: [Color::BLACK; 4],
            border_radius: [0.0; 4],
            inset: [L::Auto; 4],
            z_index: None,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            visibility: true,
            opacity: 1.0,
            color: Color::BLACK,
            background_color: Color::TRANSPARENT,
            background_image: false,
            font_family: vec!["sans-serif".into()],
            font_size: 16.0,
            font_weight: 400,
            italic: false,
            line_height: 0.0, // 0 => normal (1.2 * font-size)
            text_align: TextAlign::Left,
            text_decoration: false,
            text_decoration_line: false,
            text_transform: TextTransform::None,
            white_space: WhiteSpace::Normal,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align_top: false,
            flex_direction: FlexDirection::Row,
            flex_wrap: false,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            align_self: None,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: L::Auto,
            row_gap: 0.0,
            column_gap: 0.0,
            list_style_bullet: false,
            list_style_number: false,
            float_left: false,
            float_right: false,
            clear: false,
            before_content: None,
            after_content: None,
        }
    }

    /// Copy the *inherited* properties from `parent`.
    pub fn inherit_from(&mut self, parent: &ComputedStyle) {
        self.color = parent.color;
        self.font_family = parent.font_family.clone();
        self.font_size = parent.font_size;
        self.font_weight = parent.font_weight;
        self.italic = parent.italic;
        self.line_height = parent.line_height;
        self.text_align = parent.text_align;
        self.text_transform = parent.text_transform;
        self.white_space = parent.white_space;
        self.letter_spacing = parent.letter_spacing;
        self.word_spacing = parent.word_spacing;
        self.visibility = parent.visibility;
        self.list_style_bullet = parent.list_style_bullet;
        self.list_style_number = parent.list_style_number;
        self.cursor_inherit_marker(parent);
    }

    /// `caret-color`-like placeholders live here to keep the API small.
    fn cursor_inherit_marker(&mut self, _parent: &ComputedStyle) {}

    /// Effective line box height in px (`normal` => 1.2 × font-size).
    pub fn line_box(&self) -> f32 {
        if self.line_height > 0.0 {
            self.line_height
        } else {
            self.font_size * 1.2
        }
    }

    /// `margin: auto` centring helper.
    pub fn margin_auto(&self, side: usize) -> bool {
        matches!(self.margin[side], L::Auto)
    }

    /// Index helpers: 0 top, 1 right, 2 bottom, 3 left.
    pub const TOP: usize = 0;
    pub const RIGHT: usize = 1;
    pub const BOTTOM: usize = 2;
    pub const LEFT: usize = 3;

    /// Total horizontal padding + border.
    pub fn horizontal_decoration(&self) -> (f32, f32) {
        let left = self.padding[Self::LEFT].resolve_zero(0.0) + self.border_width[Self::LEFT];
        let right = self.padding[Self::RIGHT].resolve_zero(0.0) + self.border_width[Self::RIGHT];
        (left, right)
    }

    /// A short one-line description for `tiny --layout`.
    pub fn summary(&self) -> String {
        let d = match self.display {
            Display::None => "none",
            Display::Inline => "inline",
            Display::Block => "block",
            Display::InlineBlock => "inline-block",
            Display::Flex => "flex",
            Display::InlineFlex => "inline-flex",
            Display::ListItem => "list-item",
            Display::Table => "table",
            Display::TableRowGroup => "row-group",
            Display::TableRow => "row",
            Display::TableCell => "cell",
            Display::Contents => "contents",
        };
        format!(
            "display={d} font={:.0}px w={} h={}",
            self.font_size,
            l_debug(self.width),
            l_debug(self.height)
        )
    }
}

fn l_debug(l: L) -> String {
    match l {
        L::Auto => "auto".into(),
        L::Px(v) => format!("{v:.0}px"),
        L::Linear { pct, fixed } => format!("{pct:.0}%+{fixed:.0}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_resolution() {
        assert_eq!(L::Px(12.0).resolve(500.0), 12.0);
        assert_eq!(
            L::Linear {
                pct: 50.0,
                fixed: -8.0
            }
            .resolve(400.0),
            192.0
        );
        assert_eq!(L::Auto.resolve(300.0), 300.0);
        assert_eq!(L::Auto.resolve_zero(300.0), 0.0);
    }

    #[test]
    fn line_box_normal_is_1_2_em() {
        let mut s = ComputedStyle::initial();
        assert_eq!(s.line_box(), 16.0 * 1.2);
        s.line_height = 20.0;
        assert_eq!(s.line_box(), 20.0);
    }

    #[test]
    fn inheritance_copies_only_inheritables() {
        let mut parent = ComputedStyle::initial();
        parent.color = Color::rgb(1, 2, 3);
        parent.font_size = 20.0;
        parent.text_align = TextAlign::Center;
        parent.display = Display::Flex;
        parent.margin = [L::Px(4.0); 4];
        let mut child = ComputedStyle::initial();
        child.inherit_from(&parent);
        assert_eq!(child.color, Color::rgb(1, 2, 3));
        assert_eq!(child.font_size, 20.0);
        assert_eq!(child.text_align, TextAlign::Center);
        // non inherited
        assert_eq!(child.display, Display::Inline);
        assert_eq!(child.margin[0], L::Px(0.0));
    }

    #[test]
    fn whitespace_predicates() {
        assert!(WhiteSpace::Pre.preserves());
        assert!(!WhiteSpace::Normal.preserves());
        assert!(WhiteSpace::Normal.wraps());
        assert!(!WhiteSpace::Nowrap.wraps());
    }
}
