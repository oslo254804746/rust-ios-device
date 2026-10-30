# Gemini执行准备与通用步骤

本文件中的命令是执行模板。本次编写文档没有执行构建、依赖安装或手机操作。只对用户实际提供并登记的设备运行；所有 `<...>`、示例bundle和路径必须替换为真实、已批准的测试值。

## 固定目录、二进制和证据

使用原生 PowerShell，避免 Git Bash/MSYS 改写AFC的 `/`、Windows长路径或argv。先执行纯主机检查：

```powershell
$Repo = 'C:\Users\wangbaofeng\RustroverProjects\rust-ios-device'
Set-Location -LiteralPath $Repo
git status --short
git branch --show-current
git rev-parse HEAD
git rev-parse origin/master
git rev-list --count origin/master..HEAD

$Plan = Join-Path $Repo 'docs\testing\real-device-2026-10-28'
$RunId = Get-Date -Format 'yyyyMMdd-HHmmss'
$RunRoot = Join-Path $env:USERPROFILE "ios-device-tests\$RunId"
New-Item -ItemType Directory -Path $RunRoot | Out-Null
New-Item -ItemType Directory -Path "$RunRoot\private","$RunRoot\evidence","$RunRoot\issues","$RunRoot\tools","$RunRoot\fixtures" | Out-Null
Copy-Item -LiteralPath "$Plan\results-template.csv" -Destination "$RunRoot\results.csv"
Copy-Item -LiteralPath "$Plan\run-record.template.json" -Destination "$RunRoot\run-record.json"
$RustIos = Join-Path $Repo 'target\debug\ios.exe'
```

权限：RunRoot应只对用户/测试执行者开放，核对Windows ACL；Unix的0600声明不能替代Windows ACL。`private/` 保存原始设备ID、凭据、账号信息及原始敏感输出；`evidence/` 是审核过的脱敏副本。生成结果不写回源码目录，不提交到Git。

允许文档本身是未提交改动，但源码dirty时必须记录差异和实际文件hash；不stash/reset用户改动。测试期间不pull、不切分支、不让其它会话改受测代码。若需更新代码，结束本轮，开新run_id并重新构建。

## 基线采集

写入 `run-record.json`，原始命令输出保留本地文件：

```powershell
rustc --version
cargo --version
go version
node --version
npm --version
ffmpeg -version
git -C "$Repo\raw-projects\go-ios" rev-parse HEAD
git -C "$Repo\raw-projects\go-ios" status --short
git -C "$Repo\raw-projects\pymobiledevice3" rev-parse HEAD
git -C "$Repo\raw-projects\pymobiledevice3" status --short
```

某命令未安装就记录 `MISSING`，不把环境缺失当设备功能缺陷。Windows 的 `python.exe` 可能只是 Microsoft Store占位程序（本次文档环境即如此），先找实际Python解释器，例如 `py -3 --version` 或用户提供的venv绝对路径。不要假定任意 `python` 可用。

本次文档核对到 go-ios HEAD `ca2c56d2c294ff702cadf4821edd30b069d3a264`，pymobiledevice3 HEAD `03ba2270c0c80df77c37f65adc1967595970d31f`。pmd有未跟踪 `uv.lock`；这是现有现场，不删除/覆盖，记录其存在和hash（如使用）。执行时重新核对，不把文件夹HEAD等同实际exe/已安装Python包版本。

## 构建与依赖

只由一个执行者串行构建；两套Rust仓库不能混用产物。建议先构建可复现的debug CLI，再补绑定：

```powershell
Set-Location -LiteralPath $Repo
cargo build -p ios-cli --all-features --locked
# 仅构建C接口用例需要的DLL，当前源码而非下载旧release
cargo build -p ios-ffi --locked

& $RustIos --version
& $RustIos --help
Get-FileHash -LiteralPath $RustIos -Algorithm SHA256
```

构建失败先记录完整原始日志并检查工具链/依赖，不静默升级锁文件、换成网上最新版或使用旧exe。`target/release`和`target/debug`不同，选release则记录实际命令/路径并统一使用。绑定加载的DLL/Python module要追溯到本次源码及其hash；不把本机已装的同名旧包当本仓库。

两个上游优先使用现成、来源可核对的本地构建或venv；确实需要准备依赖时，按用户允许的网络/磁盘范围操作，完整记录安装/构建日志及版本，不更新raw-projects源码。go-ios当前go.mod要求Go1.26及toolchain go1.26.5，不能按旧README假定低版本可编译。可在 `raw-projects/go-ios` 以 `go build -o "$RunRoot\tools\go-ios.exe" .` 构建，文件名必须区别于Rust的ios.exe。pmd使用该本地checkout对应的解释器/venv；安装后用 `-m pip show pymobiledevice3` 和模块路径证明来源，禁止未说明的PyPI版本替代。

FFmpeg、Node固定客户端、独立VNC客户端、编译C所需MSVC、Python绑定所需maturin各自登记。缺一项只阻塞对应用例，继续其它独立功能。不要仅为写一行PASS重装全套环境。可先跑适用主机测试作为环境基线，但它们不能填充真机PASS。

## 设备及素材登记

1. 分别只接一台手机，人工在设置核对系统、型号和build，再 `& $RustIos list`。将真实UDID保存在 `private/devices.local.json`，例如键D1/D2，勿放共享CSV。
2. 确认D1系统<17、D2系统>=17。每台记录USB/Wi-Fi、锁屏、信任、Developer Mode、DDI和开发服务；状态不明写UNKNOWN。现有信任不足需要手机确认时，先按本轮许可处理，不自动新增配对。
3. 所有设备命令以全局参数显式选中目标：`& $RustIos --udid $U ...`，全局参数放子命令前。不用 `devices[0]`、`--device auto`、默认首台或环境变量里的历史UDID代替。
4. 再同时连接两台，分别读取ProductType/ProductVersion/BuildVersion核对不串台。别名D1/D2保持整个轮次不变。

```powershell
# private/devices.local.json由用户现场填写，格式为 {"D1":"真实UDID","D2":"真实UDID"}
$PrivateRoot = Join-Path $RunRoot 'private'
$DeviceIds = Get-Content "$PrivateRoot\devices.local.json" -Raw | ConvertFrom-Json
$D1 = $DeviceIds.D1
$D2 = $DeviceIds.D2
$U = $D1                         # 每条记录前明确本次目标
& $RustIos --udid $U info
& $RustIos --udid $U lockdown get --key ProductType
& $RustIos --udid $U lockdown get --key ProductVersion
& $RustIos --udid $U lockdown get --key BuildVersion
```

素材manifest使用 `fixture_id/本地私有路径/SHA256/大小/所属设备/是否可写/恢复方法`，至少准备：

| 素材 | 用途及缺失处理 |
| --- | --- |
| 两版同bundle、合法签名测试IPA | 普通/streaming install、upgrade、数据保留；没有IPA，安装项BLOCKED而非默认安装个人App |
| 支持两套OS的Runner/WDA、.xctestrun及Tests.xctest | 明确APP/Runner bundle、测试选择、1通过/1故意失败/1慢/1crash；签名/部署由用户提供或批准 |
| 测试app可观察UI/文本框/网络/文件 | 观察输入、截图、性能、syslog及文件hash；App容器需file-sharing/开发签名适用 |
| 0B、中文空格文件、1MiB、64MiB已知hash文件 | AFC上传/下载/中断/只读/长路径；准备原始本地只读副本防覆盖 |
| RGB/半透明RGBA PNG、RGB/灰度JPEG | 设备与主机各方向；CMYK/无效格式仅用副本做拒绝边界 |
| GPX测试轨迹 | 不涉及真实敏感位置，最后location reset |
| 测试网页与第二origin | run_id标题、iframe/跨源、console、fetch受控endpoint、可点击/输入区域；手机能访问，不能给手机用PC的localhost |
| 实际系统匹配的合法DDI/个性化镜像 | 只按批准范围挂载；无素材记BLOCKED，不能下载不明镜像/绕过签名 |
| 专属mobileconfig/provisioning、监督证书、伴侣、备份 | 条件用例专用；普通手机没有监督/手表不必为测试变更监督/配对 |
| 合法目标IPSW | 仅F06已支持的只读预检；当前没有素材则BLOCKED，完整Update继续搁置 |

## 每次操作的原始记录

有限命令可按下列格式单独执行；先创建唯一CaseDir，绝不覆盖早期attempt。`$CaseId/$Attempt`由当前CSV行确定：

```powershell
$CaseId = 'INFO-001'
$Attempt = 1
$CaseDir = Join-Path $PrivateRoot "$CaseId-D1-USB-$Attempt"
New-Item -ItemType Directory -Path $CaseDir | Out-Null
$StartedAt = Get-Date
$Watch = [Diagnostics.Stopwatch]::StartNew()
& $RustIos --udid $U info 1> "$CaseDir\rust.stdout.txt" 2> "$CaseDir\rust.stderr.txt"
$ExitCode = $LASTEXITCODE
$Watch.Stop()
# 将StartedAt、ExitCode、Watch.ElapsedMilliseconds、确切argv、截图/hash/实际观察写入metadata.json及CSV。
```

PowerShell重定向的文本编码/流转换不适合捕获原始二进制；截图/pcap/媒体使用命令自己的 `--output`，对文件单独hash。原始argv含UDID/密码时保存private，分享副本换别名，不把凭据直接作为参数给日志记录器。原始stderr不能被脱敏步骤覆盖，保存私有原件与脱敏副本的对应关系。

长流在可控独立终端启动，记录PID、端口和启动前基线，到用例时限手动Ctrl-C或产品自身有限参数结束，再等清理完成。无产品timeout的命令由执行者设置外层观察上限；外层强杀发生必须标记 `forced_kill`，不能冒充产品正常取消。只停止本次确切进程树，禁止 `Stop-Process -Name ios`、杀usbmuxd/Apple服务、清空所有隧道。表里的timeout是计划观察上限，不是断言所有设备操作必须在相同时间内完成。

接下来是 `RECIPE:` 用例的实际执行方法。

## AUTH / AUTH_HTTP / TUNNEL_HTTP

先用当前帮助确认 `tunnel auth create/check`、`tunnel serve` 的配置参数。使用RunRoot/private下专属策略和token文件，权限分别list/connect/manage-tunnels/manage-server/debug及明确设备范围；token通过文件读取，不出现在命令行或共享证据。`--local-unauthenticated`只可用于明确127.0.0.1隔离测试，不能和公网绑定混用。

Manager基准入口（独立终端，测试结束Ctrl-C）：

```powershell
& $RustIos tunnel serve --userspace --host 127.0.0.1 --port 49151 --local-unauthenticated
```

使用当前 `docs/http-boundary.md`、`docs/management-access.md`、`docs/tunnel.md` 的策略和HTTP契约，分别执行：

- `GET http://127.0.0.1:49151/tunnels`、`GET /tunnel/<真实UDID>`；列表只能观察本manager实际拥有的条目。
- `POST /start-tunnel`，JSON `{"udid":"真实目标","connection_type":"usb"}`；Wi-Fi使用源码/帮助实际接受的类型，不猜名字。核对实际连接后再算成功。
- `DELETE /tunnel/<同一UDID>`；只停本次条目，不能影响另一设备/原来运行的manager。`tunnel stop/list`是CLI访问入口，按实际--help选择manager地址/授权文件，不能把默认目标当已选中设备。
- 错token、错设备scope、过期/撤销、错误Host/Origin/非loopback绑定逐条测试；保留status及设备实际无新连接的观察。合法权限用例也需正向对照。

拥有两部但仅一部现代手机时，可测旧设备明确拒绝/新设备工作与隔离；“两个同时有效现代隧道”另记缺硬件，不伪装通过。

## WINDOWS_AFC

先完成AFC真实上传，取得 `$RemoteFile` 的已知hash和长度。每个变体使用自己的本地路径、attempt和metadata，执行 `file pull <REMOTE> <LOCAL> [--force]`：

| 变体 | 准备和核对 |
| --- | --- |
| new | 本地新路径，下载后hash/长度与fixture一致 |
| existing-no-force | 预建不同hash文件，无force拒绝且原文件不变 |
| force | 同目标带force，变为完整fixture hash |
| unicode | 中文、空格、emoji路径；记录UTF-16长度与实际文件位置 |
| long-path | RunRoot内普通绝对路径超过260 UTF-16单位，保留长度与Windows长路径配置 |
| verbatim | 对已存在路径构造真实 `\\?\C:\...`，不以普通长路径代替 |
| readonly | 私有测试文件设ReadOnly，以及目标是目录/不可写测试目录分别测；失败保留原marker/hash，随后恢复权限 |
| recovery | 错误后用新合法路径下载，完整hash与无临时残留 |
| unc | 实际用户授权 `\\server\share\...`，记录访问身份和共享可达；没有共享就BLOCKED |

勿为这组用例改全局注册表/个人目录ACL。失败对照pmd/go-ios时使用相同逻辑目标，若某上游只接受不同路径表示，说明不是同等Windows路径能力，不能直接豁免本项目路径错误。

## VIEWER_IMAGES / END_OF_RUN

用独立浏览器和标准RFB客户端逐一执行Web/VNC；不同server实例不要共用端口。图像至少覆盖设备→主机与主机→设备，PNG RGB/RGBA及JPEG RGB/灰度分别记录。手机上真正粘贴/显示，保存受控画面的截图或录屏摘要。RFB用标准ExtendedClipboard DIB能力，bit3=DIB、bit4=files；不能自拟PNG/JPEG位标。

lossless PNG核对尺寸和RGBA像素；JPEG核对可读图、尺寸/方向及预先确定的视觉或像素误差阈值，不能强求JPEG字节相同。实际VNC客户端不支持图像剪贴板时该组合BLOCKED/UNSUPPORTED，不把只传文本算图像成功。服务自己编码后解码的往返不算设备互通；物理设备与宿主剪贴板须有两端观察。

结束先关闭本次浏览器/客户端和服务，确认Display原状态、触点/按键释放、同一端口可重新监听；恢复剪贴板测试基线、定位reset、设置原值、测试app/profile/目录按批准清理。记录本次PID/监听器/接口/路由清单与前后差异。不能因输出“cleanup complete”就不看遗留，不能删除未经本轮创建的文件。

## CDP_REAL_DEVICE

源码/协议支持以 `docs/cdp-compatibility.md` 为准，当前F04仍不完整。测试网页需有2个手机可达origin、同源和跨源iframe、可读标题、可控制的fetch服务器、可观察console和输入控件；服务器日志含run_id。不要用个人网页。

1. 先 `webinspector opened-tabs`，从本次结果选page ID；对原生eval先做 `1+1` 基线。
2. 当前Rust桥 `webinspector cdp --host 127.0.0.1 --port 9222 --local-unauthenticated`。使用 `/json/list` 返回的实际WS地址/target ID，确认对应手机/页面。
3. 使用 `scripts/cdp-clients/package-lock.json` 固定Puppeteer25.12.0/Playwright1.63.0环境。新建放RunRoot下的少量驱动脚本，明确连接真实端点；**不要直接执行 `rust-bridge.mjs` 的host fixture流程就宣称真机通过**。已有脚本可借鉴方法/断言，注入服务和预设ID必须替换为真实观察。
4. 每个CASES中的Runtime/Page/Fetch/Memory等子流程分别记录请求/原始响应/事件时间与手机/网页服务器的实际结果。运行前先核对支持参数，不能在脚本中吞掉不支持命令继续假通过。
5. fetch rewrite/fulfill/fail用本轮专属测试endpoint。native metrics没有sample就等待合理采样或记原错误，不能填0。页面console缺上下文证据保留原事件，不自填context ID/主机时间。
6. Playwright默认connectOverCDP与手工低级CDP子集是两条用例。默认初始化被download拒绝时记KNOWN_GAP，不通过修改库跳过命令改变其含义。
7. Selenium创建/删除真实session、导航/标题/元素查询；pmd交互shell不是同等HTTP WebDriver服务。Input/下载目前明确不支持部分不算真实功能通过。

## XCTEST_REAL_DEVICE

两系统需各自可安装、签名匹配的测试app/runner和选择器；不要用空plan或旧报告。`--wait`与有限结果时间必须显式，至少一个实际通过测试。先执行CASES中两种CLI入口，再根据本机--help选择configuration/target、失败筛选与rerun参数。

每个流程记录：app/runner版本与hash、实际选择、安装写次数、启动PID、测试case结果/失败、terminal、JUnit计数/sidecar、syslog/crash采集和stop。故意失败测试的预期是可靠失败报告；基础设施异常不能被测试失败掩盖。取消/断线后的未知设备效果明确记录，不猜杀PID、不自动重放安装。

完整安装链需要用户先批准该IPA/bundle，然后才使用 `TestWorkflowArgs` 和当前CLI已支持的显式安装参数；不能为了绕过权限退回先默认安装。F02已有功能可以实测，整链尚未验证的地方如实记结果，不由本次计划自动关闭F02。

WDA必须实际已安装/可启动，先 /status 再创建session。session/element/PID都从本轮响应读取；截图输出新文件避免覆盖原件。对go-ios的XCTest命令，必须使用UPSTREAM.md里的反向对应关系。

## SDK_REAL_DEVICE

先静态核对当前公共接口，避免旧 `docs/python-binding.md` 文字与已实现接口不一致。C/Python方法以 `crates/ios-py/src/lib.rs`、`device_session.rs` 和 `crates/ios-ffi/include/ios_rs.h` 为准。

- Python：同一实际解释器打印 `ios_rs.__file__` 并追溯构建；在独立受控脚本中 `with ios_rs.open_device(明确UDID)`，分别调用查询/apps/files/syslog的真实方法。异步用源码中确切 `async_*` 名称，记录取消与finally close。没有脚本就BLOCKED，不把mock smoke通过作真机证据。
- 有限 `DeviceSession`：query/query_async/service/service_async、预算/cancel使用既有pair bytes及明确数字usbmux endpoint。凭据从private文件/stdin读取，不复制到代码/CSV。不要假定系统usbmux端口或从别的手机拿pair record。
- Python现代隧道：`ios_rs.start_tunnel(UDID, mode="userspace")`，观察真实metadata，`asyncio_proxy()`里连接真实RSD，finally close。测试失败后确认原本 `asyncio.open_connection` 已恢复。
- C：用当前DLL/header编译的harness，借鉴 `list_devices.c`、`device_info.c`、`device_services.c`。后者的固定 `/iTunesArtwork` 不保证设备存在，必须改为本轮批准fixture并记录harness源码/hash。read/free/close使用正确所有权，不故意制造已释放指针访问。有限会话使用实际header的函数名，不虚构ABI。
- Rust：给直接公共client/API的轻量harness明确显式目标、只读/有限操作与服务清理；外部TestWorkflowEngine::run要实际执行真实测试。没有harness或对应操作没执行，明确BLOCKED。已有 `workflow_api.rs` 等假服务测试只作host证据。

三个语言分别计数；相同服务在CLI成功不能自动填Python/C成功。Python有限会话构造为 `DeviceSession(udid, pair_record_bytes, endpoint)`，`service/service_async` 的operation值是 `apps/files/read_file/logs`；Device便捷对象的日志方法则是 `read_syslog/async_read_syslog`，二者不能混用名称。上游只能比较同设备服务语义，无法证明C ABI内存所有权或Python取消合同完全等价。

## CrashReportMover和秘密输入补充

`crash ls/show/pull/watch` 等在准备服务时可能通过mover整理设备全部待处理报告；`crash flush`本身就是设备范围pending flush，并无“只flush某个测试app”的过滤保证。即使用测试文件名筛选读取，仍须在本轮允许该设备级整理副作用；没有此范围则先记录NOT_RUN，不能把READ标签当作无副作用保证。保留其它报告，不清空个人crash。

Wi-Fi安装支持 `IOS_WIFI_PASSWORD` 环境变量，p12相关操作优先用源码声明的私密环境/输入方式；不把密码写进argv或日志。若需要临时进程环境，仅注入当前受控子进程并finally清理，不使用持久用户/系统环境变量。CSV和命令证据只记录“秘密由私密环境提供”。

每条用例的permission描述主要动作；查询若内部需要临时Userspace tunnel/服务会话，还须具备相应SESSION_WRITE范围。READ从不授权新增信任、kernel接口/路由、媒体stopAll或全设备mover等附带动作；按实际生产路线补齐前置。
