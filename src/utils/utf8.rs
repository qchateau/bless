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
