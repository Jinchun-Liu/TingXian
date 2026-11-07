#!/usr/bin/env python3
"""Normalize the user-selected research handoff. Never import it as user facts.

No network, model, credentials or app database. Source verification is the
handoff author's scoped observation, not a new independent web verification.
"""
import argparse
import hashlib
import json
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def sha(value):
    return hashlib.sha256(value).hexdigest()


def prepare(root):
    root = root.resolve()
    manifest = root / "SHA256SUMS.txt"
    for line in manifest.read_text().splitlines():
        expected, name = line.split(maxsplit=1)
        path = (root / name.lstrip("*")).resolve()
        if not path.is_relative_to(root) or not path.is_file() or sha(path.read_bytes()) != expected:
            raise ValueError(f"Handoff integrity mismatch: {name}")
    def read(name):
        return json.loads((root / name).read_text())
    sources, rules, videos = [], [], []
    for namespace, folder in [("CV", "cv"), ("INT", "interview"), ("SP", "spoken")]:
        filename = f"research/{folder}/sources.json"
        for number, original in enumerate(read(filename)["sources"]):
            source = dict(original)
            source["id"] = f'{namespace}:{original["id"]}'
            u = urlsplit(source["url"])
            source["canonical_url"] = urlunsplit((u.scheme.lower(), u.netloc.lower(), u.path.rstrip("/"), u.query, ""))
            source["duplicate_group"] = sha(source["canonical_url"].encode())
            source["handoff_locator"] = f"{filename}#/sources/{number}"
            source["verification_basis"] = "handoff_observation_not_reverified_web"
            source["namespace"] = "reference_methods"
            source["personal_fact"] = False
            source["source_material_is_instruction"] = False
            source["admission"] = "scoped_reference_only"
            sources.append(source)
        if folder != "spoken":
            for video in read(f"research/{folder}/videos.json")["videos"]:
                videos.append({**video, "id": f'{namespace}:{video["id"]}',
                               "runtime_admitted": False,
                               "reason": "Use separately registered text sources only; metadata never authorizes body claims."})
    refs = {s["id"]: s for s in sources}
    general_types = {"G01": ["role_fit"], "G02": ["introduction"], "G03": ["project"],
                     "G04": ["behavioral"], "G05": ["design", "code"], "G06": ["code"],
                     "G07": ["design"], "G08": ["unknown"], "G09": ["project"],
                     "G10": ["reverse_question"], "G11": ["retrospective"], "G12": ["complex"]}
    technical_terms = {"T01": ["null", "schema", "空值", "字段缺失"],
                       "T02": ["幂等", "重复写入"], "T03": ["504", "超时"],
                       "T04": ["pytest", "参数化"], "T05": ["flaky", "偶发", "重跑"],
                       "T06": ["缺陷", "复现"], "T07": ["回归", "优先级"],
                       "T08": ["AI辅助", "AI 辅助", "ai-assisted"]}
    for namespace, folder in [("CV", "cv"), ("INT", "interview")]:
        filename = f"research/{folder}/knowledge_cards.json"
        for number, card in enumerate(read(filename)["cards"]):
            cid = card["id"]
            role_index = namespace == "INT" and cid.startswith("R")
            action = card.get("action", card.get("advice", card.get("answer_guidance")))
            if not action:
                raise ValueError(f"Missing action: {namespace}:{cid}")
            source_refs = [f"{namespace}:{i}" for i in card["source_ids"]]
            for sid in source_refs:
                if sid not in refs:
                    raise ValueError(f"Missing source {sid}")
            exception = card.get("exception", "岗位大纲只供选材；不是企业固定流程、真题频率或本人能力证明。")
            tools = ["background"] if namespace == "CV" else ["background", "realtime"]
            scope = {"tools": tools, "modes": ["interview"] if namespace == "INT" else ["*"],
                     "question_types": general_types.get(cid, ["*"]),
                     "roles": [cid] if role_index else ["*"], "seniority": ["*"], "languages": ["*"]}
            rules.append({"id": f"{namespace}:{cid}", "version": "1", "namespace": "reference_methods",
                          "trigger": card.get("trigger", card.get("focus", card.get("title", "相关问题"))),
                          "action": action, "scope": scope, "exceptions": [exception],
                          "source_refs": source_refs, "origin": "editorial_synthesis",
                          "negative_case": "不得省略适用条件或例外：" + exception,
                          "validation_status": "index_only" if role_index and cid not in ["R01", "R07", "R08"] else "candidate",
                          "admission_check": "references_and_scope_checked_not_model_validated",
                          "query_terms": technical_terms.get(cid, []),
                          "handoff_locator": f"{filename}#/cards/{number}",
                          "personal_fact": False})
    for filename, prefix, field in [("design/spoken_response_policy.json", "SP", "rules"),
                                     ("design/realtime_policy_candidates.json", "LIVE", "policies")]:
        for number, card in enumerate(read(filename)[field]):
            source_refs = [f"SP:{s}" for s in card.get("source_refs", [])]
            source_refs += card.get("interview_card_refs", [])
            rules.append({"id": f'{prefix}:{card["id"]}', "version": "1", "namespace": "reference_methods",
                          "trigger": card.get("trigger", "spoken_answer"), "action": card["action"],
                          "scope": {"tools": ["realtime", "background"] if prefix == "SP" else ["realtime"],
                                    "modes": ["*"], "question_types": ["*"], "roles": ["*"],
                                    "seniority": ["*"], "languages": ["zh"] if prefix == "SP" else ["*"]},
                          "exceptions": ["不能覆盖事实、原文、权限、触发、取消、预算或流式终态约束。"],
                          "negative_case": "精简表达不能删除否定、数值、条件、子问题、代码或贡献限制。",
                          "source_refs": source_refs, "origin": "product_design",
                          "validation_status": "candidate", "admission_check": "design_reference_not_external_fact",
                          "query_terms": [], "handoff_locator": f"{filename}#/{field}/{number}", "personal_fact": False})
    identifiers = {r["id"] for r in rules} | set(refs)
    for rule in rules:
        if not set(rule["source_refs"]) <= identifiers:
            raise ValueError(f'Dangling reference {rule["id"]}')
    registry = {"schema_version": "tingxian-method-registry-1", "policy_version": "toolkit-1-candidate",
                "handoff_manifest_sha256": sha(manifest.read_bytes()), "harness2_enabled": False,
                "active_policy": "legacy", "sources": sources, "rules": rules,
                "role_index": read("design/role_mapping_32.json")["roles"], "videos": videos,
                "limits": {"application_context_tokens": 262144, "initial_pilot_roles": ["R01", "R07", "R08"]}}
    registry["registry_sha256"] = sha(canonical(registry).encode())
    return registry


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--handoff", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.handoff)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("x", encoding="utf-8") as output:
        output.write(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"rules": len(result["rules"]), "sources": len(result["sources"]),
                      "distinct_canonical_urls": len({s["canonical_url"] for s in result["sources"]}),
                      "active_policy": result["active_policy"], "registry_sha256": result["registry_sha256"]}))
