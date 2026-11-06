"""Generate one fictional portrait locally for avatar engineering evaluation.

Requires an already downloaded SD-Turbo Diffusers directory and CUDA. It never
downloads weights, opens a device, sends a prompt, or plays media. The fixture
generator is not the user's personal-avatar registration feature.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import time

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"
os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()
    if args.output.exists() or args.output.with_suffix(".json").exists():
        raise FileExistsError("fixture output already exists")
    if not args.model.is_dir():
        raise FileNotFoundError(args.model)
    import torch
    from diffusers import AutoPipelineForText2Image
    if not torch.cuda.is_available():
        raise RuntimeError("CUDA is required for this fixture generator")
    prompt = ("Professional studio headshot of a fictional adult East Asian man, "
              "entire head and hair visible with space above the head, chest and shoulders visible, "
              "front facing, eyes open, neutral expression, mouth closed, short black hair, "
              "plain navy crew neck shirt, gray background, soft even lighting, "
              "photograph framed from the chest up, realistic skin")
    started = time.perf_counter()
    pipe = AutoPipelineForText2Image.from_pretrained(
        args.model, torch_dtype=torch.float16, variant="fp16", local_files_only=True,
    ).to("cuda")
    image = pipe(prompt=prompt, num_inference_steps=4, guidance_scale=0.0,
                 height=512, width=512,
                 generator=torch.Generator(device="cuda").manual_seed(args.seed)).images[0]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive output creation prevents accidental replacement of an existing file.
    with args.output.open("xb") as handle:
        image.save(handle, format="PNG")
    metadata = {
        "kind": "locally_generated_fictional_test_portrait", "real_person_input": False,
        "network_inference": False, "model": "stabilityai/sd-turbo",
        "model_directory": str(args.model), "seed": args.seed, "steps": 4,
        "guidance_scale": 0.0, "prompt": prompt,
        "elapsed_seconds": time.perf_counter() - started,
        "sha256": hashlib.sha256(args.output.read_bytes()).hexdigest(),
        "torch": torch.__version__,
    }
    with args.output.with_suffix(".json").open("x", encoding="utf-8") as handle:
        json.dump(metadata, handle, ensure_ascii=False, indent=2)
    print(json.dumps(metadata, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
