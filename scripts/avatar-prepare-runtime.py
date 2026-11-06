"""Download and hash-check pinned optional avatar models; install no packages.

Run in an environment with huggingface_hub. No login is needed for these public
revisions. Weights and cache must live outside the TingXian source checkout.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path

LIVEPORTRAIT = ("KlingTeam/LivePortrait", "82a4fa6735ca58432b6ce39301b4b9ee066dea47", "liveportrait", [
    "README.md", "liveportrait/landmark.onnx",
    "liveportrait/base_models/appearance_feature_extractor.pth",
    "liveportrait/base_models/motion_extractor.pth",
    "liveportrait/base_models/warping_module.pth",
    "liveportrait/base_models/spade_generator.pth",
    "liveportrait/retargeting_models/stitching_retargeting_module.pth",
])
SD_TURBO = ("stabilityai/sd-turbo", "b261bac6fd2cf515557d5d0707481eafa0485ec2", "sd-turbo-fixture", [
    "LICENSE.md", "README.md", "model_index.json", "scheduler/scheduler_config.json",
    "tokenizer/merges.txt", "tokenizer/special_tokens_map.json",
    "tokenizer/tokenizer_config.json", "tokenizer/vocab.json",
    "text_encoder/config.json", "text_encoder/model.fp16.safetensors",
    "unet/config.json", "unet/diffusion_pytorch_model.fp16.safetensors",
    "vae/config.json", "vae/diffusion_pytorch_model.fp16.safetensors",
])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--with-fixture-generator", action="store_true")
    parser.add_argument("--verify-only", action="store_true",
                        help="verify existing files against pinned Hub metadata; do not download")
    args = parser.parse_args()
    root = args.root.resolve()
    checkout = Path(__file__).resolve().parents[1]
    if root == checkout or checkout in root.parents:
        raise ValueError("model root must be outside the source checkout")
    if args.manifest.exists():
        raise FileExistsError(args.manifest)
    os.environ["HF_HOME"] = str(root / "hf-cache")
    os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
    os.environ["HF_HUB_DISABLE_XET"] = "1"
    from huggingface_hub import HfApi, snapshot_download
    api = HfApi(endpoint="https://huggingface.co", token=False)
    sources = [LIVEPORTRAIT] + ([SD_TURBO] if args.with_fixture_generator else [])
    manifest = {"schema_version": 1, "inference": False, "files": []}
    for repo, revision, directory, files in sources:
        info = api.model_info(repo, revision=revision, files_metadata=True)
        if info.sha != revision:
            raise ValueError("model revision mismatch")
        metadata = {entry.rfilename: entry for entry in info.siblings}
        for name in files:
            if name not in metadata:
                raise ValueError(f"expected model file missing: {name}")
        local = root / "models" / directory
        if not args.verify_only:
            snapshot_download(repo, revision=revision, local_dir=local, allow_patterns=files,
                              token=False, endpoint="https://huggingface.co", max_workers=3)
        for name in files:
            path = local / name
            size = path.stat().st_size
            entry = metadata[name]
            if size != entry.size:
                raise ValueError(f"file size mismatch: {name}")
            sha256 = hashlib.sha256()
            git_blob = hashlib.sha1(f"blob {size}\0".encode("ascii"))
            with path.open("rb") as handle:
                for part in iter(lambda: handle.read(8 * 1024 * 1024), b""):
                    sha256.update(part)
                    git_blob.update(part)
            if entry.lfs:
                valid = sha256.hexdigest() == entry.lfs.sha256
            else:
                valid = git_blob.hexdigest() == entry.blob_id
            if not valid:
                raise ValueError(f"content digest mismatch: {name}")
            manifest["files"].append({"repo": repo, "revision": revision, "file": name,
                                      "size": size, "sha256": sha256.hexdigest()})
        print(f"VERIFIED {repo}: {len(files)} files", flush=True)
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    with args.manifest.open("x", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2)


if __name__ == "__main__":
    main()
