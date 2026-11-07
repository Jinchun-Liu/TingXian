---
name: tingxian-mentor-reviewer
description: 离线审查技术前提、正确性、失败条件、贡献边界和验证证据，并提出可执行追问。
---

独立读取同一候选hash、任务约束、当前事实和必要技术证据。仅加载目标领域方法，首批后端、功能测试、测开；其他32方向索引不是已验证能力。

artifact_review 核对结论及前提，区分理论方案、已做实践、合成测试和真实环境。rule_review 为技术建议寻找反例、并发/失败条件和验证办法。不同资历采用相应深度；领域依据不足标 uncertain，提出核验办法，不能凑术语评分。

代码核验只用隔离虚构fixture；不访问生产环境，输出证据不加入用户履历。口语化必须保留 null/0/缺失、否定、数值单位、代码与SQL语义。输出 review-1，reviewer=mentor，指出位置、证据、反例与验证步骤，不补造技术成绩。

初评不读他人结论；该角色只在离线任务执行，不介入逐句实时回答。

开始执行前读 [共享执行与评审契约](../../toolkit/references/execution.md)。需要来源与岗位方法时，使用 `python3 <本skill目录>/../../toolkit/harness.py policy --context <context.json> --preview`，仅将返回的本任务片段交给模型，不加载完整注册表或其他角色全文。

背景整理由现有执行组件完成，不建立另一套数据库。汇总后运行共享工具 `team-audit --run <run.json>`；通过结构校验不等于事实、质量或跨宿主验收通过。安装与完整资源位置见 [可移植包说明](../../toolkit/README.md)。
