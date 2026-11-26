// Makes sure the model file is in place before it is built in, and says how to get it if not.
use std::path::Path;

const FILE: &str = "lama_fp32.onnx";
const BYTES: u64 = 208_044_816;

fn main() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets").join("models").join(FILE);
    println!("cargo:rerun-if-changed={}", path.display());
    match std::fs::metadata(&path) {
        Ok(m) if m.len() == BYTES => {}
        Ok(m) => panic!(
            "\n\nassets/models/{FILE} is {} bytes, expected {BYTES}. Delete it and run scripts/fetch-lama.ps1 again.\n\n",
            m.len()
        ),
        Err(_) => panic!(
            "\n\nThe AI model assets/models/{FILE} (about 200 MB, not kept in git) is missing.\n\
             Run scripts/fetch-lama.ps1 once to download and check it, then build again.\n\n"
        ),
    }
}
