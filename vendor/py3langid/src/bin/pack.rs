//! Packs an upstream LIDG2 model: `pack-langid in.lidg out.lidc`.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = &args[..] else {
        eprintln!("usage: pack-langid in.lidg out.lidc");
        std::process::exit(2);
    };
    let original = std::fs::read(input).expect("read the model");
    let packed = py3langid::Model::pack_original(&original, Some(6)).expect("pack the model");
    std::fs::write(output, packed).expect("write the packed model");
}
