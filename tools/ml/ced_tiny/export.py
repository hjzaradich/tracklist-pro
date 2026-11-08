"""Rebuilds ced-tiny.onnx (and labels.json) from the original CED-tiny release.

    python -I export.py --out DIR [--download-dir DIR]

Downloads Xiaomi's CED-tiny from Hugging Face at a pinned commit, loads CED's own
code from that download unchanged, and exports the network to ONNX (opset 17) with
one input, `mel`, and two outputs:

    mel     (1, 64, 1001)  float32  CED's log-mel input for 10 s of 16 kHz audio
    logits  (1, 527)       float32  AudioSet label scores (before any sigmoid)
    emb     (1, 192)       float32  the pooled embedding the label layer reads

Hugging Face's own model.onnx has the labels only; the embedding output is our change.
Then it runs the new file with onnxruntime and checks it against PyTorch (2e-5).
See README.md. Needs the packages in requirements.txt and network access.
"""
import argparse
import hashlib
import json
import os
import sys
import tempfile

import numpy as np
import torch
import torch.nn as nn

REPO_ID = "mispeech/ced-tiny"
REVISION = "ace276d29dd0bb3f3517b0fa8cf300738c409019"
# Only what is needed: CED's code, its config, the weights. (Not the Hugging Face model.onnx.)
FILES = [
    "config.json",
    "configuration_ced.py",
    "feature_extraction_ced.py",
    "modeling_ced.py",
    "preprocessor_config.json",
    "model.safetensors",
]
OPSET = 17
INPUT_SHAPE = (1, 64, 1001)
TOLERANCE = 2e-5
ONNX_NAME = "ced-tiny.onnx"
LABELS_NAME = "labels.json"


class CedWithEmbedding(nn.Module):
    """CED-tiny's encoder, then the mean over its tokens (the embedding), then its label layer."""

    def __init__(self, model):
        super().__init__()
        self.m = model

    def forward(self, mel):
        tokens = self.m.encoder(mel).logits  # (batch, tokens, 192)
        emb = tokens.mean(1)
        logits = self.m.outputlayer(emb)
        return logits, emb


def download(download_dir):
    """The pinned files of the pinned revision, into download_dir; returns that folder."""
    from huggingface_hub import hf_hub_download

    for name in FILES:
        hf_hub_download(REPO_ID, name, revision=REVISION, local_dir=download_dir)
    return download_dir


def load(model_dir):
    """(feature extractor, CedWithEmbedding in eval mode), from CED's own code in model_dir."""
    from transformers import AutoFeatureExtractor, AutoModelForAudioClassification

    fe = AutoFeatureExtractor.from_pretrained(model_dir, trust_remote_code=True)
    model = AutoModelForAudioClassification.from_pretrained(model_dir, trust_remote_code=True).eval()
    return fe, CedWithEmbedding(model).eval()


def labels_from(model_dir):
    """The 527 label names in output order, from the model's config id2label."""
    with open(os.path.join(model_dir, "config.json"), encoding="utf-8") as f:
        id2label = json.load(f)["id2label"]
    names = [id2label[str(i)] for i in range(len(id2label))]
    assert len(names) == 527  # AudioSet repeats a few display names; the order is what counts
    return names


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def export(model, path):
    torch.manual_seed(0)
    x = torch.randn(*INPUT_SHAPE)
    with torch.no_grad():
        ref_logits, ref_emb = model(x)
    torch.onnx.export(
        model, (x,), path, input_names=["mel"], output_names=["logits", "emb"],
        opset_version=OPSET, dynamo=False,
    )
    import onnxruntime as ort

    session = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
    logits, emb = session.run(None, {"mel": x.numpy()})
    d_logits = float(np.abs(logits - ref_logits.numpy()).max())
    d_emb = float(np.abs(emb - ref_emb.numpy()).max())
    print(f"onnxruntime vs torch, max abs difference: logits {d_logits:.2e}, emb {d_emb:.2e}")
    if emb.shape != (1, 192) or logits.shape != (1, 527):
        sys.exit(f"unexpected output shapes {logits.shape} {emb.shape}")
    if max(d_logits, d_emb) > TOLERANCE:
        sys.exit(f"onnxruntime and torch differ by more than {TOLERANCE}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", required=True, help="where to write the .onnx and labels.json")
    ap.add_argument("--download-dir", help="keep the download here (default: a temporary folder)")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        model_dir = download(args.download_dir or tmp)
        _, model = load(model_dir)
        path = os.path.join(args.out, ONNX_NAME)
        export(model, path)
        with open(os.path.join(args.out, LABELS_NAME), "w", encoding="utf-8", newline="\n") as f:
            json.dump(labels_from(model_dir), f, ensure_ascii=False, indent=0)
            f.write("\n")
    print(f"{ONNX_NAME}: {os.path.getsize(path)} bytes, sha256 {sha256_of(path)}")


if __name__ == "__main__":
    main()
