# CLI 操作入口覆盖核查（2026-10-28）

此报告核对 CLI 命令树、391 条用例定义、原轮次 20261028-152926 的精确命令证据，以及 Codex 两个缺陷修复轮次的有限回归。不是一次全量补测；本次核查只运行帮助页，没有新增手机业务操作。

## 结论

| 项目 | 数量 |
|---|---:|
| 顶层 CLI 命令，与源码枚举逐项核对 | 63 |
| 规范操作入口，包括可直接运行的父命令 | 291 |
| 原轮次出现业务调用记录的入口 | 96（32.99%） |
| 完全没有业务调用证据的入口 | 195（67.01%） |
| 全部操作都没有调用记录的顶层命令 | 27 |
| 本次缺陷回归验证过有限结果的不同入口 | 5 |

96 只代表出现过调用，包括失败、超时、前置不足以及未完成预定场景，绝不等于 96 个功能通过。修复轮次的 11 次 CLI 命令来自 6 次缺陷回归和 5 次冒烟检查，共涉及 5 个入口；均已在原轮次的 96 个入口中，因此没有扩展命令树的调用覆盖范围。

## 计划与执行的区别

243 个入口在 command_template 中有直接命令模板；另外 48 个主要通过 RECIPE、标题或预期/清理描述关联。后者需要拆成独立执行明细，不能凭关联就标为已测。

逐项人工复查了自动文字匹配遗漏的 8 处：DDI uninstall/unmount、condition disable、HID button/touch、lockdown remove、tunnel auth check/stop。它们确实在组合步骤中被提到，因此不能武断称为“计划完全漏掉”。但原 COVERAGE.md 只按顶层能力与整组用例关联，无法表达 291 个子入口及其完成情况。

新的 CLI-COMMAND-CHECKLIST.csv 为全部 291 个入口各保留一行，列出直接用例、组合引用、计划权限、实际调用情况及公开选项。它补的是可核对的执行清单，不会把这些入口登记成已通过。

## 明确的用例与命令错位

| 原用例 | 计划操作 | 实际命令 | 影响 |
|---|---|---|---|
| NET-001 | `forward` | `companion list` | 伴侣缺陷是真实的，但端口转发仍缺测；伴侣列表对应 NET-008 |
| NET-004 | `notify wait` | `idam get` | 通知等待未执行，原 PASS 不成立 |
| PROFILE-003 | `profiles cloud-config` | `provisioning list` | 云配置查询未执行，原 PASS 不成立 |
| WEB-002 | `webinspector eval` | `webinspector opened-tabs` | JavaScript 求值未执行，不能借用标签页失败标 UPSTREAM_MATCH |

本次修复轮次之前沿用了报告给出的 NET-001 标签；corrected-regression-case-ids.csv 将 companion 的三次成功结果对应到 NET-008，并保留 reported_original_case_id。没有更改原轮次文件。

前次已识别的 7 条证据不足仍成立；本次进一步发现 NET-001 和 WEB-002 的错位，在 cli-audited-original-results.csv 中也暂记 INCONCLUSIVE。它是独立审核副本，不代表其它原 PASS 已全部重新验收。

## 哪些整组没有业务调用记录

`amfi`、`arbitration`、`btlogger`、`debug`、`debugserver`、`dproxy`、`erase`、`file-relay`、`forward`、`hid`、`httpproxy`、`location`、`mdm`、`memlimitoff`、`notify`、`pcap`、`power-assert`、`preboard`、`prepare`、`restore`、`runtest`、`runwda`、`runxctest`、`tunnel`、`viewer`、`wda`、`wifi`。

部分跳过符合风险或素材限制；另外一些只读项也被跳过。例如 `arbitration version`、`apps show/icons/install-record`、`notify observe/wait/stream/springboard`、`rsd observe`、`webinspector eval/cdp` 没有对应业务调用证据。READ 仍需逐项核对服务、应用、会话和素材前置，不能直接承诺现在全可执行。

`lockdown start-tunnel` 有记录，不代表 `tunnel start/serve/list/stop/auth` 也测过；`viewer --help` 不代表 Web/VNC/剪贴板测过；`ios --version` 不代表 Python/C 绑定验收；`symbols list` 不代表 pull/download 已验收。

## 全部命令族的入口调用情况

| 命令族 | 操作入口 | 有调用记录 | 无调用记录 |
|---|---:|---:|---:|
| `accessibility-audit` | 8 | 5 | 3 |
| `activation` | 6 | 3 | 3 |
| `amfi` | 2 | 0 | 2 |
| `apps` | 17 | 3 | 14 |
| `arbitration` | 3 | 0 | 3 |
| `backup` | 13 | 2 | 11 |
| `batterycheck` | 1 | 1 | 0 |
| `batteryregistry` | 1 | 1 | 0 |
| `btlogger` | 1 | 0 | 1 |
| `companion` | 5 | 1 | 4 |
| `crash` | 11 | 1 | 10 |
| `ddi` | 16 | 8 | 8 |
| `debug` | 1 | 0 | 1 |
| `debugserver` | 1 | 0 | 1 |
| `device-control` | 6 | 2 | 4 |
| `devicestate` | 3 | 1 | 2 |
| `diagnostics` | 12 | 7 | 5 |
| `discover` | 4 | 4 | 0 |
| `diskspace` | 1 | 1 | 0 |
| `doctor` | 1 | 1 | 0 |
| `dproxy` | 1 | 0 | 1 |
| `erase` | 1 | 0 | 1 |
| `file` | 9 | 4 | 5 |
| `file-relay` | 1 | 0 | 1 |
| `forward` | 1 | 0 | 1 |
| `heartbeat` | 1 | 1 | 0 |
| `hid` | 7 | 0 | 7 |
| `httpproxy` | 2 | 0 | 2 |
| `idam` | 2 | 1 | 1 |
| `info` | 5 | 5 | 0 |
| `instruments` | 17 | 6 | 11 |
| `ip` | 1 | 1 | 0 |
| `list` | 1 | 1 | 0 |
| `listen` | 1 | 1 | 0 |
| `location` | 3 | 0 | 3 |
| `lockdown` | 21 | 18 | 3 |
| `mdm` | 5 | 0 | 5 |
| `memlimitoff` | 1 | 0 | 1 |
| `mobilegestalt` | 1 | 1 | 0 |
| `notify` | 6 | 0 | 6 |
| `os-trace` | 4 | 2 | 2 |
| `pair` | 4 | 1 | 3 |
| `pasteboard` | 5 | 1 | 4 |
| `pcap` | 1 | 0 | 1 |
| `power-assert` | 1 | 0 | 1 |
| `preboard` | 2 | 0 | 2 |
| `prepare` | 2 | 0 | 2 |
| `profiles` | 6 | 1 | 5 |
| `provisioning` | 6 | 1 | 5 |
| `restore` | 10 | 0 | 10 |
| `rsd` | 3 | 2 | 1 |
| `runtest` | 1 | 0 | 1 |
| `runwda` | 1 | 0 | 1 |
| `runxctest` | 1 | 0 | 1 |
| `screenshot` | 1 | 1 | 0 |
| `springboard` | 11 | 4 | 7 |
| `symbols` | 3 | 1 | 2 |
| `syslog` | 1 | 1 | 0 |
| `tunnel` | 6 | 0 | 6 |
| `viewer` | 4 | 0 | 4 |
| `wda` | 10 | 0 | 10 |
| `webinspector` | 6 | 1 | 5 |
| `wifi` | 2 | 0 | 2 |

## 补测方式

1. 以 291 行清单逐入口推进；组合用例必须追加具体 subcheck，禁止父级 PASS 覆盖未执行子操作。
2. 先核对已跳过的只读项和可在本机完成的授权文件/本地备份/崩溃解析检查。缺素材只阻塞受影响的入口，写明缺哪一项。
3. 需要测试 app、签名 IPA/Runner、DDI、已配对手表、第二台手机、真实 Wi-Fi 等的入口，分别登记前置；不把它们统一写成“缺 fixture”。
4. 还原、重置、擦除、重启、信任、配置和系统网络等操作继续遵守原权限边界；缺授权保留 NOT_RUN，不为提高比例执行。
5. 一条业务测试至少记录规范命令、脱敏参数/私有 argv 位置、实际版本与路线、结果断言、错误/取消和清理。解析失败退出 2、帮助、版本号均不计业务调用完成。

## 方法与限制

从固定二进制递归收集 334 份帮助页，剔除内置 help，保留叶子和具有默认行为的父命令；再与源码 63 个 Commands 入口核对。早期粗扫的 288 未包含无说明文字的 eval/selenium 和可直接执行的 pair；本报告使用完整展开后的 291。

原 private/ 下有 117 份命令目录，其中两个是 help/version，另两个解析失败退出 2；其余 113 次调用涉及 96 个不同入口。别名归到规范名称，重复调用和不同参数不重复累加入口数。原始用例 ID 与操作不一致时按实际 argv 识别入口，另报错位，不用整组 capability_ids 作为执行证明。

此清单仍不是参数组合覆盖率：`apps list --app-type`、AFC `--crash`/容器、配置键、输出格式、超时/取消、iOS 版本和 USB/Wi-Fi 分支需要继续展开。291 也不包括 CDP/WDA 的全部协议方法、SDK 公共方法或语言绑定。

固定二进制 SHA256：`49d1a4dc7ba697ee378c253396ee70fbcf2d89dc2f66489a9265717fdd41b448`。
完整本地证据：修复轮次目录下 cli-coverage/，包含 cli-tree.json、逐页帮助、291 行详细 CSV、195 行未调用清单、四处错位记录及可复现的采集/分析脚本。公开清单不包含真实设备标识、凭据或个人输出。
