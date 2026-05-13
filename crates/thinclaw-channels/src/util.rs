pub(crate) fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    if s.is_char_boundary(pos) {
        return pos;
    }
    let mut i = pos;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
