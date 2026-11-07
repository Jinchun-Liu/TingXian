# Complex answers and vision acceptance

This is a frozen, fictional acceptance corpus for the production Rust context,
provider payload and SSE implementation. Preparation is the default and makes
**zero model calls**. There are 20 short questions, 10 multi-condition questions,
5 code/SQL questions and 12 synthetic images. No résumé, application database,
recording, capture permission or existing screen is read.

## Prepare

Build the production runner and draw new PNG fixtures on macOS:

```sh
cargo build --locked -p interview-core --example acceptance_case
swift validation/answer-vision/render-fixtures.swift \
  validation/answer-vision/visuals.json /tmp/tingxian-new-visuals
python3 -B validation/answer-vision/run.py \
  --binary target/debug/examples/acceptance_case \
  --images /tmp/tingxian-new-visuals --output /tmp/tingxian-new-acceptance
```

Both output directories must be new. Set the binary path to the actual Cargo
target directory when using `CARGO_TARGET_DIR`. Optional `--cases S01 L02 V04`
prepares a subset; the manifest lists the actual cases. Never overwrite a prior
run or silently rerun failed cases.

`cases/*/input.json` contains only the question, fictional facts, actual spoken
context, an explicitly separate AI suggestion when relevant, and the image.
The production context builder uses its general-purpose default preferences:
unconfirmed suggestions stay in a separate labeled field, never in actual
speech. The application's interview-specific preference omits suggestions;
this runner does not import or test every installed preference. Expected answers
and review criteria never enter the request.
`prepared.json` captures the exact prepared messages and cost assumptions.
Image-only verification codes are absent from textual prompts.

`manifest.json` freezes the binary, source revision, source files, inputs,
prepared messages, PNGs and review template by SHA-256. A dirty worktree is
explicitly recorded. `authorization.example.json` is **unapproved** and cannot
start generation. Keep the frozen manifest and review template unchanged;
write review results to a separate file.

## Authorized execution and accounting

The earlier six-case Harness trial is a different scope. Obtain explicit consent
for this corpus, its model and the combined spending cap before creating an
approved authorization file. Its manifest digest must match the frozen cases.
The prior conservative reservation is 7.643134 CNY; this is not a billing receipt.
The coordinator refuses a lower prior amount, a cap above 30 CNY or a batch whose
entire two-attempt reserve cannot fit. Preparation never reads a credential.

After authorization, supply `TINGXIAN_VALIDATION_API_KEY` only to the execution
process; never put its value in a shell command, committed file or report.

```sh
python3 -B validation/answer-vision/run.py --execute \
  --binary target/debug/examples/acceptance_case \
  --output /tmp/tingxian-new-acceptance \
  --authorization /path/to/approved-authorization.json
```

The runner is fixed to the official DeepSeek endpoint, `deepseek-flash`, explicit
DeepSeek protocol and thinking disabled. It uses the production 20-second total
deadline, 4096-token maximum output and at most two transport attempts. There
are no tools, model judges or additional coordinator retries. `/user/balance`
is queried only to verify CNY; balance amounts are not saved. A redirect is not
followed. An existing `execution/` prevents accidental duplicate execution.

On 2026-11-07, the [official pricing page](https://api-docs.deepseek.com/zh-cn/quick_start/pricing/)
lists peak Flash cache-miss input at 2 CNY/million tokens and output at 8 CNY/million.
The [vision guide](https://api-docs.deepseek.com/guides/vision/) specifies at most
1024 tokens per image. Each attempt reserves UTF-8 text bytes plus 1024 framing
tokens, 1024 image tokens when present, and the full output cap. The application
window remains 262144 with 2048 safety tokens. This is a conservative estimate,
not exact tokenizer measurement or a provider-side billing limit.

The receipt's verification date must equal the execution date. Recheck the
official sources and regenerate the binary/manifest for a later date or changed
model; do not just remove the date check. Provider price changes or billing
discrepancies require stopping and reconciling before further calls.

Before starting each child, the coordinator writes and fsyncs its two-attempt
reservation. Known usage uses the maximum observed input/output per attempt;
missing, malformed or partial usage keeps that attempt's entire reserve. A
missing terminal record keeps both reserves even if some usage arrived.
Repeated/out-of-order request IDs fail closed. Interruption, three consecutive
incomplete answers, an exceeded estimate, or uncertain completion stops further
calls. JSON transport evidence is persisted before parsing. Reservation checks
remain active under `python -O`. The actual provider bill stays unknown until
independently reconciled.

## Review

1. Check transport completion, provider usage, image SHA and the response's
   image-only code. An image code supports correct input handling but is not a
   substitute for the rest of the question.
2. Read every answer against its criteria. Record a quotation/location and
   `true`/`false` for each item in a copy of `review-template.json`; initially
   all judgments and the reviewer are null. If an assistant performs a first
   review, label it assistant review; do not call it independent human review.
3. For L01–L10, report per-case and aggregate sub-question coverage, targeting
   at least 90% per the PRD. Every critical negation/numeric condition must pass.
   Missing or unclear evidence does not count as a pass.
4. Check short-answer concision separately from long-answer coverage. Code must
   preserve missing/None/zero/empty-string/False distinctions and retry limits.
   Do not use word count, key phrase presence or valid JSON as a quality score.
5. Verify answer-first output followed by `<<<QUESTION>>>` and the complete
   corrected question. Report marker failures, incomplete streams and meaningful
   first-response timing separately from semantic quality. Do not hide failed
   cases by retrying them; changes need a new explicitly labeled run.

The images cover code, SQL, a drawn flow diagram, a table, multiple choice, long
images, small type, incomplete content, embedded instructions, bilingual terms,
context-selected adjacent questions and a blank screen. Rendering fails if a
text block exceeds its canvas. V06 and V07 deliberately test readability limits;
the model should acknowledge unreadable content rather than invent it.

These are **synthetic images sent to a real API only after authorization**.
They do not validate native capture, actual double-screen selection, permission
denial, cancellation, screen preview, installed GUI rendering, audio, three-way
parallel scheduling, cloud ASR or a 60-minute meeting. Those PRD items remain
separate. The new terminal-state regression also ensures a partial answer ended
by `aborted`, resource exhaustion, filtering, a tool request or an unknown finish
reason is retained but never marked complete, consistent with the
[official response schema](https://api-docs.deepseek.com/api/create-chat-completion/).

## Offline tests

```sh
python3 -B -m unittest discover -s validation/answer-vision -p 'test_*.py' -v
cargo test --locked -p interview-core --all-targets
```

Core stream tests use a local loopback server only. Document tests need the
project's installed Python/YAML dependencies on `PATH`. macOS CI runs these
checks without a model key. Passing them is not paid-model quality acceptance.
