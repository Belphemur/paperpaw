// Made by tools/shared-table.py from shared.list: do not edit.
#![cfg_attr(rustfmt, rustfmt::skip)]

pub(super) static TEXT: super::Addresses<44> = super::Addresses([
    paperpaw_text::measure as *const (), // TEXT_MEASURE
    paperpaw_text::is_mark as *const (), // TEXT_IS_MARK
    paperpaw_text::glyph_x as *const (), // TEXT_GLYPH_X
    paperpaw_text::draw_text as *const (), // TEXT_DRAW_TEXT
    paperpaw_text::charset::stand_in as *const (), // CHARSET_STAND_IN
    paperpaw_text::Font::from_cow as *const (), // FONT_FROM_COW
    paperpaw_text::Font::size_for as *const (), // FONT_SIZE_FOR
    paperpaw_text::Font::largest_size as *const (), // FONT_LARGEST_SIZE
    paperpaw_text::Font::line_metrics as *const (), // FONT_LINE_METRICS
    paperpaw_text::Font::advance as *const (), // FONT_ADVANCE
    paperpaw_text::Font::kern as *const (), // FONT_KERN
    paperpaw_text::Font::has_char as *const (), // FONT_HAS_CHAR
    paperpaw_text::Font::shown as *const (), // FONT_SHOWN
    paperpaw_text::BitmapFont::largest_size as *const (), // PACK_LARGEST_SIZE
    paperpaw_text::BitmapFont::line_metrics as *const (), // PACK_LINE_METRICS
    paperpaw_text::BitmapFont::has_char as *const (), // PACK_HAS_CHAR
    paperpaw_text::BitmapFont::size_for as *const (), // PACK_SIZE_FOR
    paperpaw_text::BitmapFont::glyph as *const (), // PACK_GLYPH
    paperpaw_text::BitmapFont::advance as *const (), // PACK_ADVANCE
    paperpaw_text::BitmapFont::kern as *const (), // PACK_KERN
    paperpaw_text::wrap_lines_kerned_in as *const (), // LAYOUT_WRAP_KERNED
    paperpaw_text::wrap_lines_balanced_in as *const (), // LAYOUT_WRAP_BALANCED
    crate::fonts::Fonts::handle as *const (), // FONTS_HANDLE
    crate::fonts::FontHandle::largest_size as *const (), // HANDLE_LARGEST_SIZE
    crate::fonts::FontHandle::line_metrics as *const (), // HANDLE_LINE_METRICS
    crate::fonts::FontHandle::heading_line_height as *const (), // HANDLE_HEADING_LINE_HEIGHT
    crate::fonts::FontHandle::has_char as *const (), // HANDLE_HAS_CHAR
    crate::fonts::FontHandle::advance as *const (), // HANDLE_ADVANCE
    crate::fonts::FontHandle::kern as *const (), // HANDLE_KERN
    crate::fonts::FontHandle::size_for as *const (), // HANDLE_SIZE_FOR
    crate::fonts::FontHandle::measure as *const (), // HANDLE_MEASURE
    crate::fonts::FontHandle::ink as *const (), // HANDLE_INK
    crate::fonts::FontHandle::ink_across as *const (), // HANDLE_INK_ACROSS
    crate::fonts::FontHandle::wrap as *const (), // HANDLE_WRAP
    crate::fonts::FontHandle::draw as *const (), // HANDLE_DRAW
    crate::fonts::FontHandle::draw_bolder as *const (), // HANDLE_DRAW_BOLDER
    crate::fonts::FontHandle::draw_toned as *const (), // HANDLE_DRAW_TONED
    paperpaw_text::measure_in as *const (), // TEXT_MEASURE_IN
    paperpaw_text::draw_text_in as *const (), // TEXT_DRAW_TEXT_IN
    paperpaw_text::StoredPack::open_within as *const (), // STORED_OPEN
    paperpaw_text::GlyphCache::new as *const (), // CACHE_NEW
    crate::fonts::Fonts::failed_reads as *const (), // FONTS_FAILED_READS
    crate::fonts::FontHandle::measure_in as *const (), // HANDLE_MEASURE_IN
    crate::fonts::FontHandle::draw_in as *const (), // HANDLE_DRAW_IN
]);

pub(super) static UI: super::Addresses<52> = super::Addresses([
    crate::ui::style::UiStyle::new as *const (), // STYLE_NEW
    crate::ui::style::UiStyle::noted_row as *const (), // STYLE_NOTED_ROW
    crate::ui::style::draw_focus as *const (), // STYLE_DRAW_FOCUS
    crate::ui::style::draw_separator as *const (), // STYLE_DRAW_SEPARATOR
    crate::ui::shows as *const (), // UI_SHOWS
    crate::ui::text::icon as *const (), // TEXT_ICON
    crate::ui::text::draw_icon as *const (), // TEXT_DRAW_ICON
    crate::ui::text::icon_width as *const (), // TEXT_ICON_WIDTH
    crate::ui::text::draw_icon_at as *const (), // TEXT_DRAW_ICON_AT
    crate::ui::text::heading_height as *const (), // TEXT_HEADING_HEIGHT
    crate::ui::text::draw_heading as *const (), // TEXT_DRAW_HEADING
    crate::ui::text::icon_room as *const (), // TEXT_ICON_ROOM
    crate::ui::text::draw_row_icon as *const (), // TEXT_DRAW_ROW_ICON
    crate::ui::text::tr_input as *const (), // TEXT_TR_INPUT
    crate::ui::text::draw_label as *const (), // TEXT_DRAW_LABEL
    crate::ui::text::draw_label_in as *const (), // TEXT_DRAW_LABEL_IN
    crate::ui::text::draw_text as *const (), // TEXT_DRAW_TEXT
    crate::ui::text::put_run as *const (), // TEXT_PUT_RUN
    crate::ui::text::cap_baseline as *const (), // TEXT_CAP_BASELINE
    crate::ui::text::draw_label_on as *const (), // TEXT_DRAW_LABEL_ON
    crate::ui::text::fit_pair as *const (), // TEXT_FIT_PAIR
    crate::ui::text::message_height as *const (), // TEXT_MESSAGE_HEIGHT
    crate::ui::text::draw_message as *const (), // TEXT_DRAW_MESSAGE
    crate::ui::text::note_line_height as *const (), // TEXT_NOTE_LINE_HEIGHT
    crate::ui::text::draw_note_line as *const (), // TEXT_DRAW_NOTE_LINE
    crate::ui::text::fit as *const (), // TEXT_FIT
    crate::ui::text::empty_height as *const (), // TEXT_EMPTY_HEIGHT
    crate::ui::text::draw_empty as *const (), // TEXT_DRAW_EMPTY
    crate::ui::button::draw_button_said as *const (), // BUTTON_DRAW_SAID
    crate::ui::button::button_frame as *const (), // BUTTON_FRAME
    crate::ui::button::button_label as *const (), // BUTTON_LABEL
    crate::ui::button::button_height_for as *const (), // BUTTON_HEIGHT_FOR
    crate::ui::header::Header::new as *const (), // HEADER_NEW
    crate::ui::header::Header::draw as *const (), // HEADER_DRAW
    crate::ui::header::Header::tap as *const (), // HEADER_TAP
    crate::ui::header::draw_arrow as *const (), // HEADER_DRAW_ARROW
    crate::ui::row::Row::height as *const (), // ROW_HEIGHT
    crate::ui::row::Row::draw as *const (), // ROW_DRAW
    crate::ui::row::Row::hit as *const (), // ROW_HIT
    crate::ui::label::Label::height as *const (), // LABEL_HEIGHT
    crate::ui::label::Label::draw_from as *const (), // LABEL_DRAW_FROM
    crate::ui::setting_rows::title_lines as *const (), // SETTING_TITLE_LINES
    crate::ui::setting_rows::noted_row_height as *const (), // SETTING_NOTED_ROW_HEIGHT
    crate::ui::setting_rows::draw_note_below as *const (), // SETTING_DRAW_NOTE_BELOW
    crate::ui::setting_rows::draw_value_row as *const (), // SETTING_DRAW_VALUE_ROW
    crate::ui::list::ListView::set_focus as *const (), // LIST_SET_FOCUS
    crate::ui::list::ListView::focus_change as *const (), // LIST_FOCUS_CHANGE
    crate::ui::list::ListView::paging as *const (), // LIST_PAGING
    crate::ui::list::ListView::move_focus as *const (), // LIST_MOVE_FOCUS
    crate::ui::list::ListView::row_rect as *const (), // LIST_ROW_RECT
    crate::ui::list::ListView::hit as *const (), // LIST_HIT
    crate::ui::list::list_step as *const (), // LIST_STEP
]);

/// The table of family number `n`, if the firmware shares it.
pub(super) const fn of(n: usize) -> paperpaw_table::Table {
    match n {
        0 => paperpaw_table::Table(TEXT.0.as_ptr()),
        2 => paperpaw_table::Table(UI.0.as_ptr()),
        _ => paperpaw_table::Table::NONE,
    }
}
