# 共享执行契约 v1

所有角色和原 interview-background 使用同一份库与版本。工具入口是 ../harness.py；可在宿主中串行隔离上下文，报告 serial_isolated；仅真实隔离并发才写 parallel_isolated。fixture只用于控制器测试。

一个候选版本：Lead一次生成，然后HR、Mentor、Consultant各独立一次；同一候选和来源hash，不给初评成员他人结论。最多三版、12顶层回合。最后一版也要评审；质量阈值、无改进、额度、版本上限或成员失败时停止，保留未解决问题。reviewed不是质量合格。

candidate = {id, artifact, claim_evidence_map:[{claim,evidence_refs}], gaps:[], changelog:[], policy_proposals:[]}。
review = {schema_version:"review-1", reviewer:"hr|mentor|consultant", mode:"artifact_review|rule_review|coaching", candidate_id, candidate_hash, findings:[], missing_evidence:[], prioritized_actions:[], status:"reviewed|incomplete|failed"}。
finding = {id,severity:"critical|major|minor|advisory",category:"fact|technical|fit|clarity|workflow|evidence",location,claim,evidence_refs:[],certainty:"supported|uncertain",recommended_change,verification}。supported必须有可解析证据；未知引用或旧hash不能进入汇合结果。

run = {schema_version:"tingxian-team-run-1",execution_profile:"offline",execution_mode,harness2_enabled:false,evidence_ids:[],versions:[{candidate,reviews:{hr,mentor,consultant}}],final_candidate_hash,stop_reason,attempts:[]}。stop_reason为quality_threshold/no_improvement/budget/version_limit/incomplete。candidate_hash由canonical JSON的SHA-256产生，调用共享工具digest函数，不手写。

底层API次数、重试与费用和顶层回合分开记录；attempt含id、role、candidate_id、input_tokens/output_tokens/reasoning_tokens/cost_cny（可null）。未知不能记零。宿主已有模型执行如无法取得用量也要标未知，不冒称免费或只有12次请求。共享工具无网络传输；本任务真实API默认不授权。

四命名空间：reference_methods、user_facts、task_context、improvement_experience。引用方法不能证明用户经历。原文与人工修订分开；confirmed仅表示用户核对。保留事实、贡献限制、状态、否定、数字和所有关键子问题。

实时执行只按场景/题型/岗位/语言选择相关规则，不加载四角色技能。Harness²默认关闭，只可在隔离开发集优化候选；不能读保留答案/裁判/反馈，不能改事实、权限、终态、预算或裁判。候选→验证→显式启用，结构检查不能自动晋级。
