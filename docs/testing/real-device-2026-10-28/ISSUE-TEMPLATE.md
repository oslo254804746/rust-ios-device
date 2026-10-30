# ISSUE-0000：<具体触发与可观察现象>

状态：候选 / FAIL / INCONCLUSIVE / UPSTREAM_MATCH（从中选择）\
严重度：P0/P1/P2/P3\
关联：run_id / case_id / subcheck / attempt / device_alias / transport

## 最短复现

1. 前置状态与fixture（注明所需设置/签名/真实page或PID取得方法）。
2. 精确命令或短harness脚本；UDID/凭据用别名占位，私密映射由用户另给。
3. 实际手机/产物现象；结束与清理；是否需要恢复原状态才能再次出现。

命令不要只写“安装失败”“跑用例”：参数、工作目录、超时、stdout/stderr和产物位置必须可找到。用argv数组记录含空格/中文/反斜线参数，避免shell转义歧义；不要在问题Markdown中放密码或pair bytes。

## 受测身份

| 字段 | 内容 |
| --- | --- |
| Rust commit / dirty与diff索引 | |
| Rust exe路径、SHA256、构建命令/feature | |
| Windows版本/架构、PowerShell、是否管理员 | |
| 设备别名 / ProductType / ProductVersion / BuildVersion | |
| USB/Wi-Fi / legacy/CoreDevice / 实际endpoint或RSD映射证据 | |
| 锁屏/已有信任/Developer Mode/DDI/服务前置 | |
| 允许动作及恢复范围 | |
| IPA/文件/网页/Runner等fixture_id和hash | |
| Node/FFmpeg/Python/C DLL/harness等有关依赖版本/hash | |

## 预期与实际

- 声明的支持或用例预期及来源路径：
- 实际退出码、原始错误类别/阶段、耗时：
- 实际设备效果、收到的数据、产物大小/SHA256/像素或测试case数：
- 是否仅UI/输出差异，是否涉及损坏/目标错误/迟到回复/残留：
- 成功/失败复现次数，例如 3/3；重跑的状态复位过程：
- 取消/超时/强杀是否发生，原错误与清理错误分别是什么：

## 本地上游对照

| 项目 | go-ios | pymobiledevice3 |
| --- | --- | --- |
| checkout HEAD/dirty | | |
| 实际exe或Python module来源、版本/hash | | |
| 是否有同等接口/同route，为何适用/不适用 | | |
| 精确命令或harness | | |
| 输入fixture/设备状态是否与Rust相同 | | |
| 退出码/耗时/阶段/实际设备效果 | | |
| 原始与脱敏证据索引 | | |
| 是否改变DDI/信任/隧道/安装状态 | | |

Rust→reference→Rust复查结果：\
归一化规则（只能去掉可解释的时间/PID/路径等）：\
判定：MATCH_BOTH / MATCH_GO / MATCH_PMD / RUST_ONLY_DIFFERENCE / CONFLICTING / NO_COMPARATOR / PRECONDITION_MISMATCH\
最终状态及理由：

同条件同现象应UPSTREAM_MATCH而非FAIL；缺等价/条件不齐先INCONCLUSIVE/BLOCKED。存在项目独有损坏/串台/泄漏/残留时明确说明额外差异，不用双方均报错豁免。

## 清理与现场

- 本次创建的PID、监听端口、隧道/接口/路由、服务及结束证据：
- 需要恢复的设置、模拟位置、剪贴板、app/目录/profile及实际恢复结果：
- 残留、风险、下一步允许的安全复现动作：

## 证据文件

只提交经过审核的脱敏副本。原始证据由用户private目录保存，索引可使用相对路径和hash。

```text
issues/ISSUE-0000.md
evidence/<case-device-transport-attempt>/metadata.redacted.json
evidence/<...>/rust.stdout.redacted.txt
evidence/<...>/rust.stderr.redacted.txt
evidence/<...>/go.stdout.redacted.txt
evidence/<...>/pmd.stderr.redacted.txt
evidence/<...>/artifact-hashes.json
evidence/<...>/observations.md
```

不要附用户真实pair records/证书/token、个人整机备份、原始网络流量或未审核屏幕。复现需要的真实凭据/测试素材通过用户批准的私密渠道另行提供。

## 维护者修复后复测

新commit / 新binary SHA256 / 新run_id：\
相同fixture和前置下的最小复现结果：\
必要相关回归结果：\
保留旧失败，追加新attempt/轮次，不改写历史为PASS。
