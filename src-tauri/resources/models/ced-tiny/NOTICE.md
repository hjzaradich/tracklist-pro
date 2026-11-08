# CED-tiny: notice

`ced-tiny.onnx` is the CED-tiny audio-tagging model, converted to ONNX. It is the app's optional audio model (ROADMAP 3.8) and stays off until the user switches it on.

## Source and licence

- **Model:** CED-tiny, by Xiaomi Corporation (Copyright 2023 Xiaomi Corporation and The HuggingFace Inc. team). Weights and code are licensed under the Apache License, Version 2.0. The full text is in [LICENSE](LICENSE).
- **Hugging Face repository:** `mispeech/ced-tiny`, revision `ace276d29dd0bb3f3517b0fa8cf300738c409019` (https://huggingface.co/mispeech/ced-tiny).
- **Source weights:** `model.safetensors` of that revision, 22028820 bytes, sha256 `0e086f0cd62814c6def89001f3f25193f75955696f6975ef6800af31d00d6dd7`.
- **Original code:** https://github.com/RicherMans/CED
- **Paper:** Heinrich Dinkel, Yongqing Wang, Zhiyong Yan, Junbo Zhang and Yujun Wang, "CED: Consistent ensemble distillation for audio tagging", ICASSP 2024 (https://arxiv.org/abs/2308.11957).
- **Training data:** the model was trained on AudioSet (Google).
- **labels.json:** the 527 AudioSet display names, Copyright Google, under the Creative Commons Attribution-ShareAlike 4.0 licence (CC BY-SA 4.0, https://creativecommons.org/licenses/by-sa/4.0/). Source: the AudioSet ontology (https://research.google.com/audioset/). We took the names from the CED-tiny model's `config.json` (`id2label`), which derives from the ontology, and kept their order (the model's output order). Changes: none to the names; only the format (a JSON list). This file is under CC BY-SA 4.0; the rest of this folder is under the licences stated above.

## What we changed

- We exported the network to ONNX (opset 17) from the unchanged original code and weights, using PyTorch. The export script is `tools/ml/ced_tiny/export.py`; `tools/ml/ced_tiny/README.md` says how to rebuild the file.
- Hugging Face's own `model.onnx` outputs the label scores only. Ours has two outputs: `logits` (the 527 label scores, as the original computes them) and `emb` (the 192-number embedding the label layer reads, the mean over the encoder's tokens). That extra output is the change.
- The input is `mel`, shape (1, 64, 1001): CED's own log-mel features of 10 seconds of 16 kHz mono audio. The app computes them itself (the same method as CED's `feature_extraction_ced.py`); the ONNX file does not include that step.
- Nothing else: no retraining, no fine-tuning, no quantisation. The weights are the original float32 weights.

## This file

- `ced-tiny.onnx`: 22109115 bytes, sha256 `a455633ff46e39382f5847a4b1a825b90ff40f1cc3f50c09d59f3d4ef06f0886`.
- Rebuilt on 2026-11-07 with torch 2.15.0, onnxruntime 1.31.1 (see `tools/ml/ced_tiny/requirements.txt`); a rebuild with the pinned versions gives the same bytes.
