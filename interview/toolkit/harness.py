#!/usr/bin/env python3
"""Shared offline toolkit primitives. No network client, credentials or app DB.

Compile task-specific methods, validate a bounded four-role review, and derive
traceable views from an existing background bundle. Model work stays in the
authorized host; this controller never treats a schema check as quality proof.
"""
import argparse
import copy
import hashlib
import importlib.util
import json
import time
from pathlib import Path

ROLES = ("hr", "mentor", "consultant")
NAMESPACES = ("reference_methods", "user_facts", "task_context", "improvement_experience")


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def digest(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def token_estimate(text):
    # Range, not a provider tokenizer. Upper endpoint is deliberately conservative.
    return {"lower": (len(text.encode()) + 3) // 4, "upper": len(text.encode()),
            "method": "UTF-8 byte envelope; not provider tokens"}


def load_registry(path):
    value = json.loads(Path(path).read_text(encoding="utf-8"))
    data = {k: v for k, v in value.items() if k != "registry_sha256"}
    require(digest(data) == value["registry_sha256"], "Registry hash mismatch")
    require(value["schema_version"] == "tingxian-method-registry-1", "Unsupported registry")
    require(value["harness2_enabled"] is False, "Optimizer cannot be enabled by a method registry")
    ids = {r["id"] for r in value["rules"]} | {s["id"] for s in value["sources"]}
    require(len({r["id"] for r in value["rules"]}) == len(value["rules"]), "Duplicate rule id")
    for rule in value["rules"]:
        require(rule["namespace"] == "reference_methods" and rule["personal_fact"] is False, "Method/fact mixing")
        require(set(rule["source_refs"]) <= ids, "Missing source")
        for field in ("version", "trigger", "action", "scope", "exceptions", "origin", "negative_case", "validation_status"):
            require(rule.get(field), f"Incomplete rule {rule['id']}: {field}")
    return value


def route(question, mode="interview", role=""):
    """Rule selection only. Never decides whether an utterance should trigger AI."""
    q = question.casefold()
    types = []
    patterns = {"introduction": ["自我介绍", "介绍一下自己"], "project": ["项目", "贡献", "经历"],
                "behavioral": ["冲突", "协作", "分歧", "失败经历"], "code": ["代码", "sql", "实现函数", "算法"],
                "design": ["设计", "架构", "取舍"], "unknown": ["没做过", "不知道", "不确定"],
                "complex": ["分别", "同时", "以及", "再说明"], "retrospective": ["复盘"],
                "role_fit": ["岗位", "胜任"], "reverse_question": ["反问"]}
    if mode == "interview":
        types = [name for name, terms in patterns.items() if any(t in q for t in terms)]
    rid = next((r for r, terms in [("R08", ["测开", "测试开发", "自动化", "sdet"]),
                                 ("R07", ["功能测试", "业务质量", "软件测试"]),
                                 ("R01", ["后端", "backend"])] if any(t in role.casefold() for t in terms)), "general")
    return {"tool": "realtime", "mode": mode, "question_types": types or ["general"],
            "role": rid, "seniority": "unspecified", "language": "zh", "question": question}


def compile_policy(registry, context, preview=False):
    started = time.perf_counter_ns()
    require(context.get("tool") in ("background", "realtime"), "Unknown execution profile")
    require(context.get("mode") in ("interview", "meeting", "learning", "general"), "Unknown mode")
    selected = []
    for rule in registry["rules"]:
        status = rule["validation_status"]
        if status != "validated" and not (preview and status == "candidate"):
            continue
        scope = rule["scope"]
        dimensions = [("tools", [context["tool"]]), ("modes", [context["mode"]]),
                      ("question_types", context.get("question_types", ["general"])),
                      ("roles", [context.get("role", "general")]),
                      ("seniority", [context.get("seniority", "unspecified")]),
                      ("languages", [context.get("language", "zh")])]
        if any("*" not in scope[key] and not set(values).intersection(scope[key]) for key, values in dimensions):
            continue
        if rule["query_terms"] and not any(term.casefold() in context.get("question", "").casefold() for term in rule["query_terms"]):
            continue
        # LIVE entries describe existing program invariants, never duplicate
        # the controller in the model prompt. SP11 is user-selected style work;
        # SP12/14 govern execution/evaluation, not answer wording.
        if rule["id"].startswith("LIVE:") or rule["id"] in ("SP:N11", "SP:N12", "SP:N14"):
            continue
        selected.append(rule)
    if context["tool"] == "realtime":
        methods = [r for r in selected if not r["id"].startswith("SP:")]
        # Exact topical technical rules and role-specific cards before generic
        # interview methods. This limit only selects guidance, never user facts.
        methods.sort(key=lambda r: (not bool(r["query_terms"]), r["scope"]["roles"] == ["*"], r["id"]))
        selected = methods[:6] + [r for r in selected if r["id"].startswith("SP:")]
    # Only byte-identical action + conditions + exceptions merge. Retain every
    # rule id and source map; don't merge similar-sounding but unequal rules.
    groups = {}
    for rule in selected:
        key = canonical({k: rule[k] for k in ("action", "scope", "exceptions")})
        groups.setdefault(key, []).append(rule)
    def render(items):
        lines, exceptions = [], {}
        seen = set()
        for item in items:
            identity = canonical({k: item[k] for k in ("action", "scope", "exceptions")})
            if identity in seen:continue
            seen.add(identity);lines.append(item["action"])
            exceptions[(canonical(item["scope"]),canonical(item["exceptions"]))] = '；'.join(item["exceptions"])
        return "\n".join([*lines,*exceptions.values()])
    lines, exceptions = [], {}
    for group in groups.values():
        r = group[0]
        lines.append(r['action'])
        # Exception deduplication keeps identical scope; no cross-condition merge.
        exceptions[(canonical(r['scope']), canonical(r['exceptions']))] = '；'.join(r['exceptions'])
    lines.extend(exceptions.values())
    compact = "\n".join(lines)
    # Same selected rules and conditions, not a bogus all-library baseline.
    uncompact = "\n".join(f"[{r['id']}] 条件：{r['trigger']}；范围：{canonical(r['scope'])}。{r['action']} 例外：{'；'.join(r['exceptions'])} 反例：{r['negative_case']}" for r in selected)
    identity = {"registry": registry["registry_sha256"], "context": context, "preview": preview}
    return {"schema_version": "tingxian-policy-pack-1", "policy_version": registry["policy_version"],
            "registry_sha256": registry["registry_sha256"], "pack_sha256": digest(identity),
            "preview_only": preview, "rule_ids": [r["id"] for r in selected], "text": compact,
            "method_rule_ids": [r["id"] for r in selected if not r["id"].startswith("SP:")],
            "style_rule_ids": [r["id"] for r in selected if r["id"].startswith("SP:")],
            "method_text": render([r for r in selected if not r["id"].startswith("SP:")]),
            "style_text": render([r for r in selected if r["id"].startswith("SP:")]),
            "uncompacted_same_rules": uncompact, "token_estimate": token_estimate(compact),
            "uncompacted_token_estimate": token_estimate(uncompact),
            "source_map": {r["id"]: {k: r[k] for k in ("source_refs", "handoff_locator", "origin", "scope", "exceptions")} for r in selected},
            "elapsed_ms": (time.perf_counter_ns() - started) / 1e6,
            "extra_model_calls": 0, "harness2_enabled": False}


def preparation_policy(registry, task, preview=False):
    """Task-specific background methods, never layout/ATS or all role cards."""
    allowed = {"CV:C01", "CV:C02", "CV:C03", "CV:C04", "CV:C05", "CV:C06", "CV:C09", "CV:C13", "CV:C19", "CV:C24",
               "INT:G01", "INT:G03", "INT:G08", "INT:G09"}
    context = route(" ".join(task["requirements"]), role=task["role"])
    context.update(tool="background", question_types=["role_fit", "project", "unknown"], preparation_rules=sorted(allowed))
    pack = compile_policy({**registry, "rules": [r for r in registry["rules"] if r["id"] in allowed]}, context, preview)
    if not pack["rule_ids"]: return None
    return {k: pack[k] for k in ("text", "rule_ids", "pack_sha256", "source_map", "preview_only")}


def validate_review(review, candidate, evidence_ids):
    require(review.get("schema_version") == "review-1", "Review schema mismatch")
    require(review.get("reviewer") in ROLES, "Unknown reviewer")
    require(review.get("mode") in ("artifact_review", "rule_review", "coaching"), "Unknown review mode")
    require(review.get("candidate_id") == candidate["id"] and review.get("candidate_hash") == digest(candidate), "Stale candidate review")
    require(review.get("status") in ("reviewed", "incomplete", "failed"), "Unknown review status")
    for field in ("findings", "missing_evidence", "prioritized_actions"):
        require(isinstance(review.get(field), list), f"Missing review {field}")
    require(len(review["prioritized_actions"]) <= 3 if review["reviewer"] == "consultant" else True, "Consultant has more than three actions")
    ids = set()
    for finding in review["findings"]:
        require(finding.get("id") and finding["id"] not in ids, "Missing/duplicate finding id")
        ids.add(finding["id"])
        for field, allowed in [("severity", ("critical", "major", "minor", "advisory")),
                               ("category", ("fact", "technical", "fit", "clarity", "workflow", "evidence")),
                               ("certainty", ("supported", "uncertain"))]:
            require(finding.get(field) in allowed, "Invalid finding " + field)
        for field in ("location", "claim", "recommended_change", "verification"):
            require(isinstance(finding.get(field), str) and finding[field].strip(), "Missing " + field)
        require(isinstance(finding.get("evidence_refs"), list) and set(finding["evidence_refs"]) <= set(evidence_ids), "Unknown finding evidence")
        require(finding["certainty"] != "supported" or finding["evidence_refs"], "Supported finding needs evidence")
    return copy.deepcopy(review)


def review_round(candidate, reviewers, evidence_ids):
    require(set(reviewers) == set(ROLES), "Exactly HR, Mentor, Consultant required")
    result = [validate_review(reviewers[role], candidate, evidence_ids) for role in ROLES]
    require(all(r["reviewer"] == role for r, role in zip(result, ROLES)), "Role/file mismatch")
    unresolved = [{"reviewer": r["reviewer"], **f} for r in result for f in r["findings"]]
    blockers = [f for f in unresolved if f["severity"] in ("critical", "major")]
    return {"candidate_id": candidate["id"], "candidate_hash": digest(candidate), "reviews": result,
            "unresolved": unresolved, "structurally_complete": all(r["status"] == "reviewed" for r in result),
            "blocking_findings": len(blockers), "quality_pass": False}


def audit_team(run):
    require(run.get("schema_version") == "tingxian-team-run-1", "Unknown team schema")
    require(run.get("execution_profile") == "offline", "Team cannot run in realtime")
    require(run.get("harness2_enabled") is False, "Optimizer requires a separate experiment")
    versions = run.get("versions", [])
    require(1 <= len(versions) <= 3, "One to three candidate versions required")
    require(run.get("execution_mode") in ("serial_isolated", "parallel_isolated", "fixture"), "Unknown isolation mode")
    reports = []
    seen = set()
    for version in versions:
        candidate = version["candidate"]
        require(candidate.get("id") and candidate["id"] not in seen, "Candidate ids must be distinct")
        seen.add(candidate["id"])
        for field in ("artifact", "claim_evidence_map", "gaps", "changelog", "policy_proposals"):
            require(field in candidate, "Missing Lead output " + field)
        for claim in candidate["claim_evidence_map"]:
            require(set(claim["evidence_refs"]) <= set(run["evidence_ids"]), "Unknown claim evidence")
        reports.append(review_round(candidate, version["reviews"], run["evidence_ids"]))
    require(run.get("final_candidate_hash") == reports[-1]["candidate_hash"], "Unreviewed final revision")
    require(run.get("stop_reason") in ("quality_threshold", "no_improvement", "budget", "version_limit", "incomplete"), "Missing stop reason")
    require(run["stop_reason"] != "quality_threshold" or (reports[-1]["structurally_complete"] and not reports[-1]["blocking_findings"]), "Unresolved/incomplete review cannot declare threshold")
    attempts = run.get("attempts", [])
    require(isinstance(attempts, list), "Attempt ledger required")
    ids = [a["id"] for a in attempts]
    require(len(ids) == len(set(ids)), "Duplicate attempt")
    for a in attempts:
        require(a.get("role") in ("lead", *ROLES) and a.get("candidate_id") in seen, "Attempt role/candidate missing")
        for metric in ("input_tokens", "output_tokens", "reasoning_tokens", "cost_cny"):
            v = a.get(metric)
            require(v is None or (isinstance(v, (float, int)) and not isinstance(v, bool) and v >= 0), "Invalid usage")
    return {"schema_version": "tingxian-team-audit-1", "versions": reports,
            "top_level_role_turns": 4 * len(versions),
            "bottom_level_api_requests": len(attempts) if run.get("attempts_complete") else None,
            "recorded_attempts": len(attempts), "attempts_complete": bool(run.get("attempts_complete")),
            "known_cost_cny": sum(a["cost_cny"] for a in attempts if a.get("cost_cny") is not None) if any(a.get("cost_cny") is not None for a in attempts) else None,
            "unknown_cost_requests": sum(a.get("cost_cny") is None for a in attempts),
            "quality_pass": False, "semantic_review_required": True,
            "execution_mode": run["execution_mode"], "harness2_enabled": False}


def archive_bundle(bundle, library_id, role, revisions=None, excluded=None):
    """Derived view. Desktop's v1/v2 importer remains the publication boundary.

    User assertions are distinct from source quotes; manual edits never forge
    an updated source hash. Callers must provide the latest exclusion set.
    """
    background_path = Path(__file__).resolve().parent.parent / "skills/interview-background/scripts/background.py"
    spec = importlib.util.spec_from_file_location("tingxian_background", background_path)
    background = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(background)
    background.validate_bundle(bundle)
    require(library_id and isinstance(library_id, str), "Explicit library identity required")
    background.clean(library_id, 120)
    excluded, revisions = set(excluded or []), revisions or []
    records = []
    for doc in bundle["documents"]:
        for pos, r in enumerate(doc["records"]):
            rid = digest([library_id, doc["id"], doc["hash"], pos, r])
            records.append({"record_id": rid, "namespace": "user_facts" if r["kind"] in ("personal", "evidence") else "task_context",
                            "kind": r["kind"], "claim": r["quote"], "extraction_note": r["note"], "claim_status": "source_asserted",
                            "activity_status": "unknown", "source_refs": [{"source_id": doc["id"], "source_hash": doc["hash"],
                            "extraction_revision": digest(doc), "record_index": pos, "locator": r["location"], "quote": r["quote"]}],
                            "excluded": doc["id"] in excluded, "manual_revision": None})
    by_id = {r["record_id"]: r for r in records}
    revision_ids = set()
    for rev in revisions:
        require(rev.get("record_id") in by_id and isinstance(rev.get("claim"), str) and rev["claim"].strip(), "Invalid manual revision")
        require(rev.get("basis") and rev.get("revision_id"), "Manual assertion needs revision id and user basis")
        background.clean(rev["claim"], 2000)
        background.clean(rev["basis"], 1000)
        background.clean(rev["revision_id"], 120)
        original = by_id[rev["record_id"]]
        require(not original.get("superseded") and rev["revision_id"] not in revision_ids, "Stale or duplicate manual revision")
        activity = rev.get("activity", original["activity_status"])
        require(activity in ("unknown", "planned", "in_progress", "completed"), "Invalid activity status")
        revision_ids.add(rev["revision_id"])
        original["superseded"] = True
        revised = {**original, "record_id": digest([original["record_id"], rev]), "claim": rev["claim"],
                   "claim_status": "user_asserted", "activity_status": activity, "superseded": False,
                   "manual_revision": {"id": rev["revision_id"], "basis": rev["basis"], "supersedes": original["record_id"]}}
        records.append(revised)
        by_id[revised["record_id"]] = revised
    active = [r for r in records if not r["excluded"] and not r.get("superseded")]
    # Role views are pointers; the full archive is never shortened to a summary.
    return {"schema_version": "tingxian-fact-view-2", "library_id": library_id,
            "library_version": digest(bundle), "validity_hash": digest(sorted(excluded)),
            "records": records, "role_view": {"role": role, "record_ids": [r["record_id"] for r in active], "selection_status": "unranked_requires_role_review"},
            "gaps": ["角色相关性、原文蕴含及完成状态需模型或用户复核；格式检查不证明真实性"],
            "summary": bundle["profile"], "raw_quote_tokens_estimate": token_estimate("\n".join(r["quote"] for d in bundle["documents"] for r in d["records"])),
            "deduplicated_quote_tokens_estimate": token_estimate("\n".join(sorted({r["quote"] for d in bundle["documents"] for r in d["records"]}))),
            "archive_tokens_estimate": token_estimate(canonical(records)), "summary_tokens_estimate": token_estimate(canonical(bundle["profile"])),
            "application_imported": False, "source_verified": False, "actual_model_usage": None}


def execute_fixture_team(task, adapter, max_versions=3):
    """Exercise orchestration without pretending fixture outputs are model work.

    Real host execution uses the same review-round/audit contracts. No transport
    or paid adapter is accepted here until a separately authorized runner exists.
    """
    require(getattr(adapter, "fixture_only", False) is True, "This runner accepts no model transport")
    require(1 <= max_versions <= 3, "Maximum three versions")
    run = {"schema_version": "tingxian-team-run-1", "execution_profile": "offline",
           "execution_mode": "fixture", "harness2_enabled": False,
           "evidence_ids": task["evidence_ids"], "versions": [], "attempts": [], "attempts_complete": True}
    feedback, previous_findings = [], None
    for version in range(max_versions):
        candidate = adapter.lead(copy.deepcopy(task), copy.deepcopy(feedback), version + 1)
        # Reviewers get the same candidate; neither others' output nor the
        # previous reviewer inputs can mutate the candidate or shared evidence.
        reviews = {role: adapter.review(role, copy.deepcopy(candidate), copy.deepcopy(task)) for role in ROLES}
        result = review_round(candidate, reviews, task["evidence_ids"])
        run["versions"].append({"candidate": candidate, "reviews": reviews})
        feedback = result["unresolved"]
        signatures = sorted((f["reviewer"], f["severity"], f["category"], f["claim"]) for f in feedback)
        if not result["structurally_complete"]:
            reason = "incomplete"
        elif not result["blocking_findings"]:
            reason = "quality_threshold"  # declared stopping rule, not quality evidence
        elif signatures == previous_findings:
            reason = "no_improvement"
        elif version + 1 == max_versions:
            reason = "version_limit"
        else:
            previous_findings = signatures
            continue
        run["stop_reason"] = reason
        break
    run["final_candidate_hash"] = digest(run["versions"][-1]["candidate"])
    return {"run": run, "audit": audit_team(run)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    compile_cmd = sub.add_parser("policy")
    compile_cmd.add_argument("--registry", type=Path, default=Path(__file__).with_name("registry.json"))
    compile_cmd.add_argument("--context", type=Path, required=True)
    compile_cmd.add_argument("--preview", action="store_true")
    team = sub.add_parser("team-audit")
    team.add_argument("--run", type=Path, required=True)
    archive = sub.add_parser("archive")
    archive.add_argument("--bundle", type=Path, required=True)
    archive.add_argument("--library-id", required=True)
    archive.add_argument("--role", default="")
    archive.add_argument("--revisions", type=Path)
    archive.add_argument("--excluded", type=Path)
    for action in ("preparation-plan", "preparation-input", "preparation-collect"):
        command = sub.add_parser(action)
        command.add_argument("--bundle", type=Path, required=True)
        command.add_argument("--library-id", required=True)
        command.add_argument("--task", type=Path, required=True)
        command.add_argument("--revisions", type=Path)
        command.add_argument("--excluded", type=Path)
        command.add_argument("--context-tokens", type=int, default=262144)
        command.add_argument("--input-cap", type=int, default=32768)
        command.add_argument("--output-reserve", type=int, default=8192)
        command.add_argument("--preview-methods", action="store_true")
        if action == "preparation-input": command.add_argument("--part", type=int, required=True)
        if action == "preparation-collect":
            command.add_argument("--proposals", type=Path, required=True)
            command.add_argument("--output-format", choices=("json", "markdown", "team"), default="json")
            command.add_argument("--execution-mode", choices=("fixture", "host_model", "unreported"), default="unreported")
    args = parser.parse_args()
    read = lambda p: json.loads(p.read_text(encoding="utf-8"))
    if args.action == "policy":
        result = compile_policy(load_registry(args.registry), read(args.context), args.preview)
    elif args.action == "team-audit":
        result = audit_team(read(args.run))
    elif args.action.startswith("preparation-"):
        spec = importlib.util.spec_from_file_location("tingxian_preparation", Path(__file__).with_name("preparation.py"))
        preparation = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(preparation)
        archive = archive_bundle(read(args.bundle), args.library_id, "",
                                 read(args.revisions) if args.revisions else [], read(args.excluded) if args.excluded else [])
        task = read(args.task)
        preparation.task_spec(task)
        method_pack = preparation_policy(load_registry(Path(__file__).with_name("registry.json")), task, args.preview_methods)
        plan = preparation.plan(archive, task, args.context_tokens, args.input_cap, args.output_reserve, method_pack)
        if args.action == "preparation-plan": result = preparation.manifest(plan)
        elif args.action == "preparation-input": result = preparation.input_packet(plan, args.part)
        else:
            result = preparation.collect(plan, read(args.proposals), args.execution_mode)
            if args.output_format == "markdown":
                print(preparation.render(result))
                return
            if args.output_format == "team": result = preparation.team_candidate(result)
    else:
        result = archive_bundle(read(args.bundle), args.library_id, args.role,
                                read(args.revisions) if args.revisions else [], read(args.excluded) if args.excluded else [])
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
