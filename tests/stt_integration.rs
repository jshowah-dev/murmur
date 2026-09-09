use murmur_lib::{cleanup, config::Config, dictionary::Dictionary, stt::Recognizer};
use sherpa_onnx::Wave;
use std::path::PathBuf;

fn model_dir() -> Option<PathBuf> {
    let p = Config::default().model_dir_path();
    p.join("encoder.int8.onnx").exists().then_some(p)
}

#[test]
fn fixtures_transcribe_to_expected_after_cleanup() {
    let Some(dir) = model_dir() else {
        eprintln!("model not installed; skipping");
        return;
    };
    let rec = Recognizer::load(&dir, 4).expect("load model");
    let dict = Dictionary::seed();
    let cfg = Config::default();
    let expected = std::fs::read_to_string("tests/fixtures/expected.txt").expect("expected.txt");
    let mut failures = Vec::new();
    for line in expected.lines().filter(|l| !l.trim().is_empty() && l.contains('\t')) {
        let (file, want) = line.split_once('\t').expect("tab-separated");
        let known = file.starts_with('#');
        let file = file.trim_start_matches('#');
        let want = want.replace("\\n", "\n");
        let wave = Wave::read(&format!("tests/fixtures/{file}")).expect("read wav");
        assert_eq!(wave.sample_rate(), 16_000, "{file} must be 16 kHz");
        let raw = rec.transcribe(wave.samples());
        let got = cleanup::clean(&raw, &dict, &cfg);
        if got == want {
            if known {
                eprintln!("{file}: known failure now passes; drop its # in expected.txt");
            }
        } else if known {
            eprintln!("{file}: known failure, got {got:?} (raw {raw:?})");
        } else {
            failures.push(format!("{file}: want {want:?}, got {got:?} (raw {raw:?})"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
