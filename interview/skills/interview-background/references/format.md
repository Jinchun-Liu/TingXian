# 背景资料格式与版本

工具包版本见 `../VERSION`，应用版本独立。机器可读规范为 [bundle.schema.json](bundle.schema.json)。UTF-8 JSON，包不超过 16 MiB；最多 2000 份文件，每份最多 16000 条记录。`background.py validate --bundle ...` 校验字段、长度、重复标识和疑似凭证，不证明原文或结论真实。`pack` 还会重读当前来源、核验哈希及连续引文，应用中仍须人工核对背景。

| 对象 | 字段 |
| --- | --- |
| 顶层 | `format`、`documents`、`profile`；v2 另需 `settings` |
| documents[] | `id`、`name`、`hash`、`records`；id/hash 为64位小写SHA-256，id 是本机绝对路径标识哈希，hash 是原文件字节哈希；换目录需重新 plan，不手改旧id |
| records[] | `kind`、`location`(180字)、`quote`(650字)、`note`(450字)；kind 为 personal/evidence/practice/job/technical |
| profile | `overview`(8000字，非空)、`role`(200字)、`aliases`(2000字) |
| settings | 候选 `role`(120字)、`organization`(120字)、`context`(1000字)、`terminology`(1000字)、`call_names`(240字)；无依据留空 |

名称是相对显示路径，不能含空或点开头的路径段。条目保留原材料位置与原文；说明不要补造经历。控制字符、凭证及未知字段被拒绝。长度按 Unicode 字符数，界面编辑器可能采用更保守的 UTF-16 限制。

`profile.json` 是中间文件，含 profile 三字段，可另有 `settings`。`pack` 将 settings 提到顶层并写入 `meetily-background-v2`；省略 settings 则继续写 `meetily-background-v1`。新听弦同时读取两版；旧听弦如不识别v2，可从中间 profile 去掉 settings 后重新 pack 为v1，不能仅改版本字符串。降级只丢弃设置建议，来源与背景仍保留。

设置建议不含接口、Key、录音开关、外发许可、参会模式或事实确认。导入资料不自动应用建议；应用前展示当前值，用户可编辑并逐项选择。资料版本或手动值变化时重新核对；点名称呼单独保存于本机，不改变当前人数模式。

附带 `assets/example/source.txt` 和 `profile.json` 全部是虚构案例：保留旧材料与新说明的冲突，不以文件时间判定哪份真实；接口练习不代表生产经验。`scripts/self_test.py` 会按运行目录生成对应 id/cards，不预置与机器路径绑定的 id。
