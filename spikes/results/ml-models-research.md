# Local audio ML: GPL-compatible models (research, 2026-09-27)

The requirement is on-device models that are legal to ship or download for a GPL-3.0 app. That rules out non-commercial (NC), no-derivatives (ND) and evaluation-only weights. Targets: "sounds like this" embeddings, genre/vibe/mood *suggestions*, and energy. CPU only, ONNX Runtime (`ort`), 10k–100k tracks.

The research was done by a background agent checking 25+ models. **CPU costs marked "est." are estimates, not measurements.** Benchmark before committing.

## Ruled out (non-commercial or restricted weights)
MERT, MuQ/MuQ-MuLan (CC-BY-NC), **all Essentia models** (Discogs-EffNet, MAEST, genre/mood heads: CC BY-NC-SA; Essentia itself is AGPL), OMAR-RQ (CC BY-NC-SA), CLaMP 3 (built on MERT), TTMR++ (NC), M2D/M2D-CLAP (evaluation-only). → **The one family with fine DJ-style labels (Discogs' 400 styles) isn't usable.**

## Usable candidates
| Model | Gives | Size / cost | Weights license | Notes |
|---|---|---|---|---|
| **LAION-CLAP `larger_clap_music`** | 512-d audio+text embedding; zero-shot tags from text ("dark rolling liquid DnB") | audio part ~88M; est. 1–3 s per 30 s on desktop CPU | Apache-2.0 (HF) / CC0 (lukewys checkpoints) | Music-specific; community ONNX exports exist. **Top pick for similarity plus text search.** |
| **EfficientAT `mn10_as`** (or `dymn10`) | 960-d embedding + 527 AudioSet tags, incl. House, Techno, Dubstep, DnB, Electro, UK garage, Trance, Ambient, Pop, Rock, moods (Happy, Sad, Exciting, Angry…) | ~5M params; ≤0.25 s per 10 s even on a Raspberry Pi 4 | MIT (implicit: weights in the MIT repo's releases) | Proven ONNX, and already run from Rust. **Cheap enough for 100k tracks. Top pick for tagging plus a first-pass embedding.** |
| CED tiny/mini/small/base (Xiaomi) | AudioSet tags + embedding | 5.5M–86M; the authors say tiny beats MobileNets on x86 | Apache-2.0 | Official ONNX export script. Alternative to EfficientAT. |
| Dasheng-base (Xiaomi 2024) | 768-d self-supervised embedding (strong on music) | 86M; est. 1–3 s | Apache-2.0 | Needs its own export. |
| MATPAC / MATPAC++ (2025) | 3840-d embedding; a music checkpoint exists | ViT-base-ish | Apache-2.0 (verify on the HF card) | Needs its own export. |
| PANNs Cnn14 | 2048-d + AudioSet tags | ~80M; est. 0.6–1.5 s | CC-BY-4.0 (attribution required) | Older but solid. |
| OpenL3, YAMNet, VGGish, MusiCNN | older embeddings/tags | small | CC BY 4.0 / Apache-2.0 / ISC | Weaker. MusiCNN's training data is a concern (MagnaTagATune). |
| MusicFM (FMA variant) | music SSL embedding | ~330M | MIT | Heavy. |
| Magenta RT MusicCoCa (2025) | 768-d music-text embedding | unknown | CC BY 4.0 | Promising, untested as a standalone embedder. |
| Microsoft CLAP | 1024-d embedding | 690 MB | MS-PL on HF (FSF: GPL-incompatible) vs CC BY 3.0 on Zenodo | **Unclear.** Only the Zenodo copy might be usable. |

## Recommendation
- **Embeddings:**
  1. **EfficientAT `mn10_as`** for every track (cheap, with tags in the same pass).
  2. **LAION-CLAP music**, run lazily or at low priority, for "sounds like" and text search.
- **Tag suggestions:** map EfficientAT's AudioSet genre and mood classes onto the app's genre tree as *suggestions*. Use CLAP zero-shot prompts for finer labels (riddim, liquid, tech house), with accuracy still unverified. Longer term, train small per-user heads on the user's own genres and My Tags, over the permissive embeddings. A DJ's existing My Tags and their assignments are plenty of training data.
- **Energy (1–10):** **no permissive pretrained model exists.** Build it from DSP features (loudness, onset density, spectral flux, BPM), optionally blended with a small regressor on embeddings, and calibrate by percentile within the user's own library.

## Risks
- **Training data:** most usable weights are trained on AudioSet (YouTube audio with no license) or LAION-Audio-630k ("research purposes" download terms). That's common industry practice but a legal grey area. Get legal input before shipping any model download.
- **Implicit or conflicting weight licenses** (EfficientAT, PaSST, BEATs; msclap). Pin commits, mirror the weights with a NOTICE file, and credit CC-BY models in the app.
- **Preprocessing parity:** the mel/STFT front-end in Rust must match PyTorch exactly. Use golden-vector tests.
- **Throughput:** CLAP at about 1.5 s per track is about 40 CPU-hours for 100k tracks. Use incremental background indexing, cheap model first.

## Sources
- LAION-CLAP: https://github.com/LAION-AI/CLAP · https://huggingface.co/laion/larger_clap_music · https://huggingface.co/lukewys/laion_clap · https://github.com/LAION-AI/audio-dataset/blob/main/laion-audio-630k/README.md
- EfficientAT: https://github.com/fschmid56/EfficientAT · https://github.com/fschmid56/EfficientAT/releases · https://lorenzschmidt.com/rust-inference/ · https://arxiv.org/html/2509.14049v1
- CED / Dasheng: https://github.com/RicherMans/CED · https://huggingface.co/mispeech/ced-base · https://github.com/XiaoMi/dasheng
- MATPAC: https://github.com/aurianworld/matpac · PANNs: https://zenodo.org/records/3987831 · OpenL3: https://github.com/marl/openl3
- Ruled out: https://huggingface.co/m-a-p/MERT-v1-95M · https://github.com/tencent-ailab/MuQ · https://essentia.upf.edu/models.html · https://github.com/nttcslab/m2d
- Microsoft CLAP: https://github.com/microsoft/CLAP · https://zenodo.org/records/8378278 · https://www.gnu.org/licenses/license-list.html
- AudioSet: https://research.google.com/audioset/download.html · https://research.google.com/audioset/ontology/electronic_music_1.html
