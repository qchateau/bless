pub fn find_nth_or_last<T: Eq>(data: &[T], char: T, nth: usize) -> Option<(usize, usize)> {
    let mut last_found = None;
    let mut cnt = 0 as usize;
    for (idx, data_char) in data.iter().enumerate() {
        if char == *data_char {
            last_found = Some((cnt, idx));
            if nth == cnt {
                break;
            }
            cnt += 1;
        }
    }
    return last_found;
}

pub fn rfind_nth_or_last<T: Eq>(data: &[T], char: T, nth: usize) -> Option<(usize, usize)> {
    let mut last_found = None;
    let mut cnt = 0 as usize;
    for (idx, data_char) in data.iter().enumerate().rev() {
        if char == *data_char {
            last_found = Some((cnt, idx));
            if nth == cnt {
                break;
            }
            cnt += 1;
        }
    }
    return last_found;
}
