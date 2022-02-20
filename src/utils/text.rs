use num_integer::div_ceil;
use std::{
    borrow::Cow,
    str::{from_utf8, from_utf8_unchecked},
};

pub fn decode_utf8(data: &[u8]) -> Cow<str> {
    match from_utf8(data) {
        Ok(string) => Cow::Borrowed(string),
        Err(e) => {
            if e.valid_up_to() > data.len() - 4 {
                Cow::Borrowed(unsafe { from_utf8_unchecked(&data[..e.valid_up_to()]) })
            } else {
                String::from_utf8_lossy(data)
            }
        }
    }
}

pub fn convert_tabs(mut lines: Vec<Cow<str>>, tab_width: usize) -> Vec<Cow<str>> {
    for cow_line in lines.iter_mut() {
        if !cow_line.contains('\t') {
            continue;
        }

        if tab_width == 0 {
            *cow_line.to_mut() = cow_line.replace("\t", "");
            continue;
        }

        let parts: Vec<String> = cow_line
            .split("\t")
            .map(|x| {
                let width = div_ceil(x.len() + 1, tab_width) * tab_width;
                format!("{:width$}", x, width = width)
            })
            .collect();
        *cow_line.to_mut() = parts.join("");
    }
    lines
}
