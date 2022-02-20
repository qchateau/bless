pub fn nth_or_last<I: Iterator>(mut iter: I, nth: usize) -> Option<(I::Item, usize)> {
    let mut cnt = 0;
    let mut res = None;
    while cnt <= nth {
        match iter.next() {
            Some(item) => {
                res = Some(item);
                cnt += 1;
            }
            None => break,
        }
    }

    return match res {
        Some(item) => Some((item, cnt - 1)),
        None => None,
    };
}
