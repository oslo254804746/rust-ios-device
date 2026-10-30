# 结果与上游对照判定

先记录本项目真实结果，再判断最终状态。退出0、JSON可解析、HTTP200、日志文件存在、能力枚举存在、主机fixture通过，都不能单独证明真机功能成功。

## 最终状态

| 状态 | 判定 | 是否计项目失败 |
| --- | --- | --- |
| PASS | 所有适用子步骤达到具体预期，设备/产物真实，清理正常 | 否，计实际成功 |
| UPSTREAM_MATCH | 本项目未达用户功能目标，但适用上游在严格相同前置和语义下可复现相同限制/现象，且本项目没有额外错误或残留 | **否**；单列上游共同现象，不能写PASS |
| FAIL | 本项目在已满足前置的已支持范围内出现可重现额外差异；适用上游成功，或项目独有的串台/损坏/残留/安全或契约违例有独立证据 | 是，需issue及复现证据 |
| INCONCLUSIVE | 现象不稳定、两个对照结论矛盾、无法证明同条件，或无等价实现且缺独立违例证据 | 否，待判定 |
| BLOCKED | 适用功能缺app/签名/DDI/驱动/服务条件/凭据/真实harness等，无法有效开始 | 否，列可解除前置 |
| NOT_APPLICABLE | 已证实该系统/平台/硬件不适用，如旧系统的现代专用服务 | 否，注明精确依据 |
| UNSUPPORTED | 当前实现明确不提供某个非已交付范围或命令/协议映射；不是执行成功 | 否，列支持边界；若声明支持却拒绝须继续调查 |
| KNOWN_GAP | 已知且明确搁置的缺开发范围，例如F04下载/可信Input、F06完整Update | 否，单列欠账，不提高交付率 |
| NOT_RUN | 尚未执行、缺授权、主动搁置或时间不足 | 否，但绝不能写“全部真机验收完成” |

`rust_status/go_status/pmd_status`各自填写 `PASS/FAIL/TIMEOUT/CANCELLED/UNSUPPORTED/BLOCKED/NOT_APPLICABLE/NOT_RUN`，忠实记录该工具原始行为；`final_status`由上表判定。允许本项目 raw 状态为 FAIL 而最终为 UPSTREAM_MATCH。不要修改/美化既有 `scripts/regression.py` 报告中的原始FAIL；在本轮CSV关联原报告后补最终归因。

缺许可用 `NOT_RUN + NOT_AUTHORIZED`；缺素材/驱动用 `BLOCKED + MISSING_FIXTURE/DRIVER_MISSING`。环境未齐与源码明确无支持是不同事实。

## “一致”必须满足的条件

1. 同一手机、ProductType、OS完整版本/build、锁屏和信任/Developer Mode/DDI状态。
2. 同一个协议或等价服务路线：USB/Wi-Fi、legacy/RemoteXPC、原生/Userspace/Kernel、RSD endpoint一致；不能用不同route的一个成功/失败随意豁免。
3. 同一输入：IPA hash、已安装版本、文件路径/内容、测试选择、网页表达式/页面、profile身份；输出写不同私有路径以免互相覆盖。
4. 相同权限与依赖。排除独占服务被另一个工具占住、错误token、坏证书、错误page/PID、上游旧二进制等因素。
5. 顺序独立执行：Rust A → 适用reference B → Rust A复查。每次结束原服务并确认设备回到可比较状态；涉及写操作不允许无授权重复。不得同时占用同端口/媒体/runner。
6. 比较错误阶段、类别、结果、实际设备效果和清理。错误字符串、JSON键顺序、语言、绝对路径、PID和时间戳不同可以归一化；空数据、错误目标、丢字段、截断文件、迟到成功和残留任务不能归一化掉。

日志必须记录实际预算，不为了做成“上游同样超时”给reference不合理短期限。上游比本项目慢、重试策略不同，应比较可实现功能及合理有限等待，另外记录耗时/预算合同；不能从单次时间差直接推断根因。

## 两个reference如何使用

- 失败后先核对 `UPSTREAM.md` 中语义最接近的本地实现；两个都适用且有同等能力时均对照。某一工具无此功能，登记 `NO_COMPARATOR`，不得假造一个近似操作成功/失败来填表。
- 只有一个适用且完全等价，已可复现相同现象时，另一个明确N/A/NO_COMPARATOR也可判UPSTREAM_MATCH；必须写清排除依据。
- 一个适用上游成功、另一个失败时，不能挑失败者直接给UPSTREAM_MATCH。先查版本/协议/依赖差异；无法解释先INCONCLUSIVE，证实本项目在成功上游同条件下仍失败再FAIL。
- 本项目先失败，上游运行后Rust成功：可能是上游挂载DDI/改变信任/启动隧道或设备状态被修复。记 `INCONCLUSIVE + STATE_CHANGED_BY_REFERENCE`，补记录前置，不能宣布项目修复或共同失败。
- 两上游也失败，但本项目误报成功、选错设备、损坏文件或留下独有服务/输入状态：这是额外差异，不能豁免。
- 无上游等价且仅仅“没工作”：INCONCLUSIVE。无上游等价但实证本项目吞错、越权、损坏、错误目标或违反自己声明的格式/期限/清理合同，仍可FAIL；证据需单独说明，不能仅引用需求想象。
- 已知不支持的负向用例可记录拒绝检查完成，但其“功能可用性”仍UNSUPPORTED/KNOWN_GAP，不能凭预期拒绝提高真机成功数。

## 例子

| 实际情况 | 最终判定 |
| --- | --- |
| 同一未签名IPA三工具同在安装验证阶段拒绝，无额外残留 | UPSTREAM_MATCH，记录签名限制 |
| Rust拉取测试文件截断，上游同route/hash成功，Rust A复查仍截断 | FAIL，附三次字节数/hash与最小文件 |
| D1是iOS16，CoreDevice Display不适用 | NOT_APPLICABLE，不把旧机基础截图也跳过 |
| D2有服务，但Windows没Wintun/管理员权限 | BLOCKED，不能说整个Windows不支持 |
| pmd只截图成功，Rust VNC输入没效果 | 截图不能对照VNC输入；输入先INCONCLUSIVE或有独立违例证据才FAIL |
| 手机没配对手表，companion list空且合理 | 空列表契约可PASS；真实get/listen/forward场景BLOCKED |
| Playwright默认连接被未实现download命令拒绝 | KNOWN_GAP；不能改客户端吞错后标默认初始化通过 |
| runner故意失败的测试正确产出失败JUnit/终态和非成功退出 | 故意失败用例可PASS；真实应用测试结果仍是失败，两个层次分开 |
| 用户Ctrl-C后命令正确报取消并清理，实际安装状态未知 | 取消合同用例可PASS；不能宣称安装完成/回滚 |

## 建议reason_code和严重度

`MISSING_FIXTURE`、`DRIVER_MISSING`、`SERVICE_UNAVAILABLE`、`TRUST_REQUIRED`、`DEVELOPER_MODE_OFF`、`DDI_REQUIRED`、`LOCKED_DEVICE`、`IOS_VERSION_SCOPE`、`HOST_PLATFORM_SCOPE`、`NO_COMPANION`、`NO_COMPARATOR`、`REFERENCE_UNAVAILABLE`、`REFERENCE_VERSION_UNPROVEN`、`STATE_CHANGED_BY_REFERENCE`、`MATCHED_UPSTREAM_LIMITATION`、`KNOWN_F02/F04/F06_GAP`、`NOT_AUTHORIZED`、`TIME_BUDGET_EXHAUSTED`、`REPRODUCIBLE_PROJECT_DIFFERENCE`、`CLEANUP_FAILED`、`WRONG_DEVICE`。可以扩展，但必须同时写自然语言原因。

严重度只对已证实问题分级：P0=数据损坏/误操作其它设备/凭据泄漏/整机或关键系统影响；P1=已支持关键流程不可用或明确资源/停止违例；P2=局部功能错误/可绕过；P3=输出/提示问题。立即停止进一步写入的P0要保存现场，不能为补齐上游表继续破坏状态。

## 汇总

分别统计HOST、D1/USB、D1/WIFI、D2/USB、D2/WIFI、DUAL；所有追加attempt保留，但汇总“唯一计划行”用最终审定的最新有效attempt，另列复现频率，避免重跑虚增通过数。空subcheck是父计划行；有明细时父行只承载整体结果，不把父行和子明细一起累加实际成功。子操作/格式/方向覆盖另外统计，并列尚未执行的明细。

实际成功数只算真机PASS；UPSTREAM_MATCH单列且不计FAIL。可报告 `PASS / (PASS+FAIL+UPSTREAM_MATCH)` 作为已判定有效执行中的功能达成比例，但同时列全部状态数和原计划行数，不能排掉阻塞/未运行后宣称100%覆盖。组合步骤不齐时父用例不能PASS。
