//! Packs a Model2Vec model for metldr: `pack-model model.safetensors tokenizer.json out`.
//! The float vectors become int8 rows with one scale each, which loses no measured retrieval
//! quality (notes: research/r13-static-embeddings.md).

use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let [weights, tokenizer, out] = &args[..] else {
        eprintln!("usage: pack-model model.safetensors tokenizer.json out");
        return ExitCode::FAILURE;
    };
    match pack(weights, tokenizer) {
        Ok(packed) => match fs::write(out, packed) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("could not write {out}: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn pack(weights: &str, tokenizer: &str) -> Result<Vec<u8>, String> {
    let bytes = fs::read(weights).map_err(|e| format!("{weights}: {e}"))?;
    let n = u64::from_le_bytes(bytes.get(..8).ok_or("no header")?.try_into().unwrap()) as usize;
    let header: serde_json::Value = serde_json::from_slice(&bytes[8..8 + n]).map_err(|e| e.to_string())?;
    let tensor = &header["embeddings"];
    if tensor["dtype"] != "F32" {
        return Err("the embeddings are not F32".into());
    }
    let dim = tensor["shape"][1].as_u64().ok_or("no shape")? as usize;
    let (start, end) = (tensor["data_offsets"][0].as_u64().ok_or("no offsets")? as usize, tensor["data_offsets"][1].as_u64().ok_or("no offsets")? as usize);
    let vectors: Vec<f32> = bytes[8 + n + start..8 + n + end]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let json: serde_json::Value = serde_json::from_slice(&fs::read(tokenizer).map_err(|e| format!("{tokenizer}: {e}"))?).map_err(|e| e.to_string())?;
    let vocab = json["model"]["vocab"].as_object().ok_or("no WordPiece vocabulary")?;
    let mut tokens = vec![""; vocab.len()];
    for (token, id) in vocab {
        let id = id.as_u64().ok_or("bad id")? as usize;
        *tokens.get_mut(id).ok_or("id out of range")? = token;
    }
    if tokens.iter().any(|t| t.contains('\n')) {
        return Err("a token holds a line break".into());
    }
    let unk = vocab.get("[UNK]").and_then(|v| v.as_u64()).ok_or("no [UNK]")? as u32;
    if vectors.len() != tokens.len() * dim {
        return Err("vectors and vocabulary disagree".into());
    }
    Ok(metldr_embed::pack(&tokens, unk, dim, &vectors))
}
