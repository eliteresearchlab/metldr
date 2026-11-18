# py3langid

Source: https://github.com/markusmobius/rust-py3langid, crate `rust-py3langid` 0.4.1, BSD-3-Clause
(LICENSE), itself a port of py3langid (langid.py by Marco Lui and Timothy Baldwin).

Changes from upstream:
- `model.rs`: the model in compact form: int8 weights with one scale per feature, and the byte
  automaton as each row's differences from the start row (a 256-bit mask and its targets, found
  by population count). About 25 MB in memory instead of 97 MB; on the papluca
  language-identification test set the answers match upstream's (99.14% on sentences, 93.62%
  against 93.65% on four-word spans).
- `original.rs`: the upstream LIDG2 reader, kept only to convert a model (feature `pack`, binary
  `pack-langid`).
- `preprocess.rs`: case tests and lowercasing through the standard library instead of ICU (the
  same Unicode data, about 100 KB less wasm).
- `identifier.rs`: transitions through `Model::next`, scores from int8 weights; the embedded model
  and file helpers removed (the extension downloads the packed model).
- Left out: the embedded model file, upstream tests, benchmarks and examples.
