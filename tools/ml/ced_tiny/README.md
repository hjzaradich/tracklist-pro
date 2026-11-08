# CED-tiny: rebuilding the audio model file

The app's audio model is Xiaomi's CED-tiny (Apache-2.0), converted to ONNX with one extra output, the embedding. This folder rebuilds that file from the original and writes the golden outputs the parity tests use. Nothing here runs in the app or in CI; it is a tool for whoever changes or re-checks the model.

The result is checked in at `src-tauri/resources/models/ced-tiny/` (see `NOTICE.md` there for source, revision, sha256 and what we changed).

## Rebuild

```
python -m venv venv
venv/Scripts/python -m pip install -r requirements.txt          # Windows; venv/bin/python elsewhere
venv/Scripts/python -I export.py --out some/empty/folder
```

`export.py` downloads the pinned Hugging Face repo (`mispeech/ced-tiny`) at the pinned commit, loads CED's own code from it unchanged, and writes `ced-tiny.onnx` (opset 17; input `mel` (1, 64, 1001); outputs `logits` (1, 527) and `emb` (1, 192)) and `labels.json` (the 527 label names in output order). It then runs the new file with onnxruntime and stops with an error if it differs from PyTorch by more than 2e-5. It prints the file's size and sha256.

With the pinned versions on Windows (Python 3.12) it produced exactly the checked-in file (sha256 `a455633f...6f0886`). Other torch or onnx versions, or another operating system, may write different bytes for the same model, so **the hash is not the test**. The test is parity: run `golden.py` with the new file and compare its numbers with the checked-in fixtures (below).

## Golden outputs

```
venv/Scripts/python -I golden.py --out some/folder --download-dir some/download --onnx some/empty/folder/ced-tiny.onnx
```

writes the files in `src-tauri/tests/fixtures/audio_model/` (their README says what is in them). The script is deterministic: running it twice gave identical files. With `--onnx` it also prints how far onnxruntime is from PyTorch on the golden mel (expect about 1e-5 on the logits, 2e-6 on the embedding).

To update the model: change `REVISION` in `export.py`, rebuild, run `golden.py`, replace the files, update `NOTICE.md` (revision, size, sha256), and tell the Rust side its pinned sha256 changed.

Everything here uses synthetic signals. No recording goes into the repository.
