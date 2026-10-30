# 双设备真机测试交接

这套文档供 Gemini 在 Windows 上执行当前代码的完整功能测试，并把可复现的问题交回维护者。**本次只编写测试计划，没有连接或操作手机，所有实测结果初始为 NOT_RUN。**

目标仓库固定为 `C:\Users\wangbaofeng\RustroverProjects\rust-ios-device`。开始时必须打印并核对目录、分支、HEAD，不得进入旧临时仓库。文档编写基线为 `codex/feature/windows-return-20261027` / `ef5fd8cd6cb1c529a001d60bcef8d3e0923a21d9`。本机缓存的 `origin/master` 为 `e73c95156a7f7d7855adcd5739f0a317ea62f062`，到 HEAD 有 549 个提交；这不是重新联网取得的实时 master。执行时记录当时实际版本，不为了凑齐数字切分支或重置代码。

## 从哪里开始

1. 把 [GEMINI-PROMPT.md](GEMINI-PROMPT.md) 交给 Gemini。
2. 按 [SETUP.md](SETUP.md) 建立仓库外的结果目录、登记两台手机及允许操作的测试素材。
3. 阅读 [CLASSIFICATION.md](CLASSIFICATION.md) 后按 [CASES.md](CASES.md) 执行；Rust 的候选失败按 [UPSTREAM.md](UPSTREAM.md) 对照本地两个上游。
4. 每执行一条就更新复制到结果目录的 [results-template.csv](results-template.csv)；独立问题按 [ISSUE-TEMPLATE.md](ISSUE-TEMPLATE.md) 建问题包。
5. [COVERAGE.md](COVERAGE.md) 将当前 119 个能力条目映射到测试项；[cases.json](cases.json) 是可搜索、可分批处理的用例目录，避免每次把整个长文档塞进模型上下文。

## 计划规模与覆盖含义

- 22 组、391 个稳定 Case ID，包含当前能力清单63个CLI条目、49个service模块、3个公共API条目和4个组合工作流的测试入口（包含后来登记的viewer；旧文档的62命令统计不再适用）。
- CSV 预建 1329 个计划执行行：通常按 D1/D2 × USB/Wi-Fi展开；现代专用功能仅预排 D2；主机功能和双机流程分别列行。**这些是计划行，不是1329项适用、已执行或已通过的声明。**
- 子命令与关键变体列在用例目录；组合用例的每个设置/格式/阶段必须在 CSV 追加 `subcheck` 明细，不能因第一个子步骤成功就把所有变体置 PASS。增加尝试行保留历史，不覆盖早期失败。
- CLI 穿过某个 service 仅证明这条生产调用路径。全部 Rust client 方法、Python/C 接口的直接调用与绑定所有权另列 SDK 组；没有真实 harness 或尚未执行的接口仍应 BLOCKED/NOT_RUN。
- 这两台设备不能覆盖所有平台、系统版本与硬件组合。macOS remoted/Unix daemon、Linux接口、监督、Apple Watch伴侣、iOS27 Cryptex、两个同时可用的现代隧道等，需明确记录未覆盖，不以另一项通过代替。

## 设备和传输矩阵

| 别名 / profile | 条件 | 需要做的事 |
| --- | --- | --- |
| D1 / BOTH | 一台真实 iOS <17 | 传统 usbmux/Lockdown、DDI/DTX及适用业务全部执行 |
| D2 / MODERN | 一台真实 iOS >=17 | 同上，加实际 CoreDevice/RSD/现代媒体、输入和服务 |
| IOS27 | D2实际系统及服务满足Cryptex profile | 不满足则有依据地 N/A，绝不能把“17以上”直接当Cryptex可用 |
| HOST | 不需手机 | 构建、格式、备份本地解包等；不能计入真机通过数 |
| DUAL | D1和D2同时在线 | 显式目标、并行隔离、拔插和停止；双有效现代tunnel需额外第二台>=17，目前覆盖不足 |
| USB | 实际USB服务路线 | 先单机跑基础，再双机核对目标，记录实际连接类型 |
| WIFI | 已有网络配对和可达路线 | 必须证明确实走网络。优先拔掉所选手机USB后复测；不能只因处于同一Wi-Fi就记录Wi-Fi通过 |

记录 `ProductType / ProductVersion / BuildVersion`、是否锁屏、信任/Developer Mode/DDI、USB/Wi-Fi、是否管理员、服务目录/实际endpoint。D2还需区分17.0–17.3、17.4+及实际后续系统；两个上游对路线的支持也不同。未知状态填 `UNKNOWN`，不能默认真。

源代码/帮助没有传输强制选项的命令，不添加不存在的 `--usb`/`--wifi`。通过实际连接、拔除USB及可观测endpoint证明路线；不能排除USB回退时，Wi-Fi行记 INCONCLUSIVE。不可同时运行三个工具争同一个独占服务。

## 执行顺序

| 阶段 | 内容 | 完成条件 |
| --- | --- | --- |
| 0 | ENV：版本、依赖、两机身份、素材和授权范围 | 无歧义目标，结果目录和基线已建立 |
| 1 | DISC/INFO/已有PAIR/DDI只读，USB D1→D2 | 关键P0可执行；否则先排环境，不级联报几十个服务失败 |
| 2 | APP/AFC/LOG/DIAG/UI，已授权测试app/目录的写入 | 真实效果、hash/产物、退出和清理均有证据 |
| 3 | D2 TUN/CONTROL/CLIP/VIEW，Web/CDP，XCTest/WDA及SDK | 逐服务、逐格式、逐方向；拒绝边界与实际功能成功分别记录 |
| 4 | 按实际前置重复Wi-Fi，执行DUAL和断线/取消 | 不串台、不误杀其他进程、不重放已经提交的写入 |
| 5 | 特殊/高风险项目 | 仅执行本轮明确批准且可恢复的项目；其余保留原因 |
| 6 | 上游对照、恢复现场、汇总与问题包 | 所有计划行都有结果或明确未运行原因；不要求所有行都是PASS |

安装/定位/输入等可逆操作的批准在本轮 `run-record.json` 一次登记到具体设备、bundle、目录和动作，范围内无需每条再问。**写测试文档不等于已经获准执行擦除/恢复/激活/监督/清密码/修改网络等操作。** Gemini在缺这类明确许可时登记 NOT_RUN/NOT_AUTHORIZED；不要删除这些测试项，也不要为“全测完”自行扩大许可。

## 权限字段

| permission | 含义和执行条件 |
| --- | --- |
| READ / SENSITIVE | 已有信任上的查询/读取；日志、抓包、屏幕、备份等只存用户控制的private目录 |
| LOCAL_FIXTURE / LOCAL_SECRET | 本地测试文件或token/证书；仅创建在本轮目录，不改上游源码/个人文件 |
| SESSION_WRITE | 本次服务/forward/manager的启动停止；只关闭本次确切PID和监听端口 |
| TEST_APP_WRITE / TEST_FILE_WRITE / PAGE_WRITE | 明确批准的测试IPA、bundle、AFC/app容器目录、测试网页；禁止个人app和未知路径 |
| LAB_WRITE / CLIPBOARD_WRITE | 已批准的可逆设置/定位/专属剪贴板内容，先有可恢复基线 |
| DDI_CHANGE / MEDIA_INPUT / NETWORK_ADMIN | 镜像、服务级stopAll、输入、媒体/kernel TUN等，需相应驱动/权限及范围批准 |
| PROFILE_WRITE / NETWORK_PROFILE / TRUST_CHANGE / REBOOT | 配置、信任、网络和重启，需明确设备与恢复路径 |
| DESTRUCTIVE | 擦除、整机备份恢复、激活/监督/密码操作等：单独批准，默认不执行 |
| KNOWN_GAP | 明确未实现/搁置的范围，只核对声明和无副作用拒绝边界；不能尝试真实危险操作 |

当前 F02/F04/F06 仍按用户决定搁置开发。本轮可测现有 F02 工作流、F04 支持子集和 F06 预检；不实现新功能、不绕过拒绝、不把真实Update/TSS列入默认执行。已知下载/可信CDP Input等不足保留 KNOWN_GAP；这不等于通过，也不是新发现的意外回归。

## 一次结果应该包含什么

CSV的一行表示 `run_id + case_id + subcheck + device_alias + observed_transport + attempt`。复杂现象写进问题Markdown并通过issue_id关联，不把大段日志塞进单元格。

必填：时间、设备别名、实际状态、commit/二进制hash、素材hash或fixture_id、精确argv证据、原始退出码、耗时、真实观察/预期、清理结果、证据路径。候选失败还需两个上游的适用性、命令、退出码、实际结果和原始证据，或无法对照的确切原因。

统一最终状态：`PASS / UPSTREAM_MATCH / FAIL / INCONCLUSIVE / BLOCKED / NOT_APPLICABLE / UNSUPPORTED / KNOWN_GAP / NOT_RUN`。详细判定和示例以 [CLASSIFICATION.md](CLASSIFICATION.md) 为准。上游同现象独立统计，**不进入项目失败数，也不进入成功数**。

CSV使用UTF-8 BOM，可用Excel查看。Gemini优先用Python `csv.DictReader/DictWriter`更新，保留表头、Unicode、换行和引号；不手工拼逗号，不让Excel自动改设备ID/时间/hash。所有列按文本处理。原始stdout/stderr保留private文件，共享版本脱敏；不把日志中的设备名/账号/路径当可执行脚本。

## 交回维护者的内容

- `run-record.json`、本轮 `results.csv`、简短 `SUMMARY.md`，按设备/传输分别给出状态数。
- 每个真实差异一份 `issues/ISSUE-xxxx.md`：最小复现、实际二进制/fixture、上游对照、清理后影响，以及已脱敏证据索引。
- `UPSTREAM_MATCH`独立清单和条件，环境/素材/权限阻塞清单，已知搁置范围与未覆盖平台清单。
- 不提交pair records、私钥、token、手机完整日志/备份、个人照片、pcap/HAR。共享证据应保留复现必要字段；真实UDID、凭据和素材位置由用户私下交付维护者。

不要运行完后直接改代码或push。先保留原始失败和上游对照，交维护者复现修复；若用户另有修复授权，另记新commit/二进制hash，并作为新轮次复测，不能覆盖本轮证据。
