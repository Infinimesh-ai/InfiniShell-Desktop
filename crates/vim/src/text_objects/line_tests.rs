use super::*;

#[test]
fn inner_line_excludes_surrounding_whitespace() {
    let text = "one\n \tαβ  \nthree";

    assert_eq!(vim_inner_line(text, 7), Some(6.into()..8.into()));
}

#[test]
fn inner_line_is_none_for_blank_lines() {
    let text = "one\n \t \nthree";

    assert_eq!(vim_inner_line(text, 5), None);
    assert_eq!(vim_inner_line("", 0), None);
}

#[test]
fn inner_line_uses_the_line_containing_the_cursor() {
    let text = "  one  \n two \nthree";

    assert_eq!(vim_inner_line(text, 10), Some(9.into()..12.into()));
}

#[test]
fn all_lines_covers_the_buffer() {
    let text = "one\n \nthree";

    assert_eq!(vim_all_lines(text), Some(0.into()..11.into()));
}

#[test]
fn inner_line_handles_empty_final_lines() {
    assert_eq!(vim_inner_line("one\n", 4), None);
    assert_eq!(vim_inner_line("one\n\t ", 5), None);
}

#[test]
fn all_lines_preserves_empty_lines_at_both_ends() {
    assert_eq!(vim_all_lines("\ntext\n"), Some(0.into()..6.into()));
    assert_eq!(vim_all_lines(""), Some(0.into()..0.into()));
}
