use lazy_static::lazy_static;
use std::collections::HashMap;

pub fn word_entropy(word: &str) -> f64 {
    lazy_static! {
        static ref PROBABILITIES: HashMap<char, f64> = {
            let mut p: HashMap<char, f64> = HashMap::new();
            p.insert('A', 0.0651738);
            p.insert('B', 0.0124248);
            p.insert('C', 0.0217339);
            p.insert('D', 0.0349835);
            p.insert('E', 0.1041442);
            p.insert('F', 0.0197881);
            p.insert('G', 0.0158610);
            p.insert('H', 0.0492888);
            p.insert('I', 0.0558094);
            p.insert('J', 0.0009033);
            p.insert('K', 0.0050529);
            p.insert('L', 0.0331490);
            p.insert('M', 0.0202124);
            p.insert('N', 0.0564513);
            p.insert('O', 0.0596302);
            p.insert('P', 0.0137645);
            p.insert('Q', 0.0008606);
            p.insert('R', 0.0497563);
            p.insert('S', 0.0515760);
            p.insert('T', 0.0729357);
            p.insert('U', 0.0225134);
            p.insert('V', 0.0082903);
            p.insert('W', 0.0171272);
            p.insert('X', 0.0013692);
            p.insert('Y', 0.0145984);
            p.insert('Z', 0.0007836);
            p.insert(' ', 0.1918182);

            // add some custom probabilities built empirically
            for char in '0'..'9' {
                p.insert(char, 0.01);
            }

            // we need to normalize after adding our custom probs
            let norm: f64 = p.iter().map(|(_, p)| p).sum();
            p.into_iter().map(|(c, p)| (c, p / norm)).collect()
        };
    }

    let mut entropy = 0.0;
    for char in word.to_uppercase().chars() {
        let p = PROBABILITIES.get(&char).unwrap_or(&0.1918182);
        entropy += p * p.log(PROBABILITIES.len() as f64);
    }
    return -entropy;
}
