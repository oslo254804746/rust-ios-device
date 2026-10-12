# WDA 接力修复与范围复核 · 2026-10-12

工作区与分支沿用交接，HEAD 为 `c98c819153e5fe3f12f4c31845f6026f089a7492`。
所有源码仍未提交，无推送、发布、部署或真实设备变更。本轮未派发子agent，
Rust/Cargo 串行，日志单独写文件。Q04及全部子项排除，有效分母30保持。

## 交接核验与开发范围

上一轮最终manifest的571个产品/脚本路径、11个验收日志及旧CLI二进制hash
全部匹配。旧证据保持其原范围：Core1641通过；CLI初始1305通过/2失败/6忽略，
后续仅Doctor48通过/1忽略；协议方法合并41通过/1 POSIX跳过，crash修正后单独
复验6通过。没有把这些分段证据写成最终全量重跑。

按原路线图385–407行核对L01/L02，并核对Q02/F01/F02的原定义与现有代码：

- L02已有设备serial协调、4连接许可、1秒B隔离fixture、USB身份替换、完整/失败/旧
  paired snapshot的不同规则、持久Stop/Manual及全局clear/epoch重置。已知发布元数据
  64 MiB charge由表与原native owner共同持有；源码和对应已通过测试本轮未变。
  `FailureReceipt.first/latest`、abandoned scan原错误仍以原对象保留，不透明payload
  与跨子系统历史元数据未完整归因到同一全局字节规则，L02仍待完整收口。
- L01尚有其他协议、startup/upgrade/blocking/长流及公有helper整体future被丢弃的
  异步收尾边界；本轮WDA修复不能关闭全部profiles。
- Q02四类observer和adapter均有生产接线；Windows rich clipboard watch仍直接
  NOT_RUN，因为缺owned console signal与graceful cleanup工具。这是开发缺口，
  不能只称为测试未运行。paired-identity完整CLI/mDNS、kernel Display/HID及其它
  RSD模式实际CLI验证另列待验。
- F01的USB/route/remoted两点观察、挂载/签名与legacy/existing-RSD具体操作已实现；
  完整目标DDI manifest/签名证据仍未收口。TLS1.3 legacy raw切换明确Unsupported，
  不是TLS1.3兼容完成。原物理/平台验证仍保留。
- F02原安装owner、日志、结果/JUnit、失败历史、重跑和crash路径已有子项证据；
  本轮补WDA实际CLI，现代全工作流及原平台/设备范围继续核查。

本轮保持22项已交付、7项仍缺开发、L02待完整核查，不以测试方法数升级主项。

## WDA 变更与实际证据

启动future返回ActiveTestPlan的同一次poll若跨过绝对期限，原包装器会丢弃成功值，
外部finally失去原PID/DTX owner。现在在被保护的poll内部先保存owner再判定期限；
迟到成功返回原Elapsed，finally仍持有原owner。新增`runwda --once`，一个转发连接
结束后提交原runner PID的kill请求、关闭原服务并等待原task/native owner收尾。

`scripts/test_regression_wda.py`经实际新版CLI、独立现有信任目录、localhost
TLS Lockdown、DTX和TCP peers验证启动→TCP端口ready→转发→finally。成功peer收到
精确HTTP字节和原kill请求，原PID集合为空；readiness probe与服务workers分别要求
严格EOF、原线程join。RST或fixture强制关socket不能供给PASS。TCP可连接只表示
该端口ready；startExecuting的单向写入不是设备完成ACK。此证据不是物理WDA证明。

| 本轮门禁 | 实际结果 |
| --- | --- |
| WDA Rust回归（含same-poll迟到owner） | 6通过/0失败/0忽略 |
| 新CLI native build | PASS |
| CLI严格Clippy，全目标/全feature | PASS |
| WDA完整localhost实际CLI | 5方法通过，包括3个startup错误子例、ready超时、单向start无ready、bind失败 |
| 共享fixture受影响adapter/crash回归 | 35通过/1 POSIX跳过；含helper与实际CLI，用例级边界见日志 |
| 新CLI实际能力目录 | 119条通过 |

新版CLI SHA256：`b720cfce86ea2c61575480c73979f57a72c6ff8f0c89c304ea486870b049b62f`。
源码、日志和工作树hash及最终格式/diff检查见外部`wda-validation-summary.json`。
本轮没有重跑Core/完整CLI/SDK运行时/独立feature/MSRV矩阵，沿用未变源码的
旧检查仅按其原范围陈述；新版二进制不能自动继承旧二进制的设备或运行时证据。

## 失败历史与复现

首次`python`命令命中Windows Store alias，exit9009，未执行测试；改用bundled Python。
第一次native WDA suite 1方法通过/3方法失败，成功ready后触发30秒watchdog。
原fixture没有处理转发setup的CoreDeviceProxy探测，直接抛内部错误并保留TLS socket；
补真实`InvalidService`回复，实际CLI原fallback继续走usbmux。第二次focused诊断仍
失败的日志保留。初始start故障用例还错误期待startup同步观察one-way start之后的协议错误，现独立验证
无监听端口不能满足ready，并保留kill/EOF/join断言。最终5方法全部实际重新运行通过。

复现（指定精确已构建CLI，使用隔离合成peer，不操作真实设备）：

```powershell
$env:IOS_REGRESSION_CLI = (Resolve-Path target/debug/ios.exe).Path
$env:PYTHONPATH = (Resolve-Path scripts).Path
& 'C:\Users\wangbaofeng\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe' -m unittest test_regression_wda -v
```

完整日志与manifest：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a12df9-97f9-7771-bc97-dc1b21bc779e\repair`。
旧资源baseline managed worktree、overlay patch、失败制品和uv缓存保持保留。
