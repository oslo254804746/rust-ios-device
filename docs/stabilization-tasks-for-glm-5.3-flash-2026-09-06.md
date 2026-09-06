# rust-ios-device 稳定性诊断优先任务书

> 交付对象：glm-5.3-flash
>
> 任务书版本：2026-09-06
>
> 仓库基线：32f3867cab4d937a4e8fe962227fe1de06b52081
>
> workspace version：0.1.14

## 开场执行指令

你是本任务书的执行代理。请把下列四个诊断主题按依赖顺序落实为可审查的最小代码、测试和 CI 变更，并在每一项结束时留下可复核证据：

1. 为此前没有上限的 H2/XPC 初始化增加受控的初始化期限，同时保持 TCP 连接期限、RSD fallback 和公开 API 的兼容语义。
2. 修正发布工作流的有向依赖，确保真正需要的质量门禁完成后才会构建、冒烟和上传 release、wheel、sdist 或 crate 产物。
3. 让 Python 与 C FFI 的独立构建显式带上已宣称支持的 tunnel-kernel feature，并用不需要设备和管理员权限的构建证据确认它确实编入。
4. 先用可控的半头、半正文取消场景判断 XPC 连接取消后能否复用；只有证据确认失步时才做最小修复，并保持 RSD fallback、消息路由和已有 poison 合同正确。

开始前只做以下预检，不要 reset 或 checkout 覆盖用户工作树：

~~~
git status --short
git rev-parse HEAD
~~~

若 HEAD 与本任务书中的基线不同，记录实际 HEAD、工作树变化和由此增加的不确定性，继续在现状上工作；不要把恢复到旧提交当作任务步骤。

先读本任务书引用的入口文件和仓库现有测试，再按依赖顺序实施。测试可在执行代理的环境中运行，但本任务书的撰写不代表本轮已执行任何产品修复、编译、测试、发布或真机验证。

## 范围、权限和事实边界

### 本任务书的定位

当前撰写阶段只交付任务书，不执行产品修复。用户将本文交给执行代理并要求开始执行后，执行代理应在下面明确列出的范围内自主实现、添加针对性测试并修改 CI，无须对常规可逆的实现步骤逐项申请确认；仍须遵守当时有效的权限和更高层指令。每项交付必须以 diff、测试输出或可复核日志说明实际做了什么。

执行范围包括 host 侧 Rust、Python/FFI feature wiring、GitHub Actions 门禁和可控协议单元测试。不得仅凭本文操作真机、申请管理员权限、访问发布凭据、提交、推送或发布。跨平台 runner 是验证环境：可在用户另行授权的正常 CI 流程中验证；本机没有对应环境时记录 NOT_RUN，不能伪造成功，也不应因此阻止 host 修复。

不要要求无视任何有效的更高层指令。不要创建 doctor 命令、全新错误体系、1.0 API 设计或全面真机兼容矩阵；可以在最后列出简短后续建议，但它们不是本轮必做项。

### 已确认的基线事实

以下事实来自基线代码和已完成的宿主检查，执行代理应把它们当作起点，而不是未经验证的推断：

| 事实 | 证据入口 | 对任务的约束 |
|---|---|---|
| ConnectedDevice::connect_xpc_service_with_metadata 在 crates/ios-core/src/device/connected.rs:310-336 建立 tunnel 后直接调用 XpcClient::connect_stream。 | connected.rs:323-327 | 该路径不能再让 H2 SETTINGS 或 XPC 初始化无限等待。 |
| XpcClient::connect 在 crates/ios-core/src/xpc/client.rs:25-49 已分别给 TCP dial 和初始化包上 TUNNEL_CONNECT_TIMEOUT；connect_stream 在 :52-63 自身没有期限。 | xpc/client.rs:25-63 | 不要把已有 TCP 行为误写成缺陷；要给 stream 路径补边界并避免每阶段重置同一个预算。 |
| H2Framer::connect 在 crates/ios-core/src/xpc/h2_raw.rs:249-271 写 preface、SETTINGS、WINDOW_UPDATE 后在 :274-289 等待服务端 SETTINGS；read_raw_frame 在 :293-326 对 9-byte header 和 payload 分别 read_exact。 | h2_raw.rs:249-326 | H2 header、SETTINGS 或 payload 停住时必须有可测试的上限。 |
| open_rsd_proxy_framer 在 crates/ios-core/src/device/rsd_connect.rs:194-232 取得 proxy stream 后裸调用 H2Framer::connect。 | rsd_connect.rs:207-229 | RSD 的 H2 建立也不能无界挂起，但不能用一个粗暴总超时破坏 fallback。 |
| RSD proxy 目前有 queued 3s bootstrap + 4s handshake、legacy 3s bootstrap + 3s handshake、passive 2s，并会重新打开 framer。 | rsd_connect.rs:41-190 | 任何共享 deadline 方案都必须说明 queued→legacy→passive 的兼容性；默认不要把所有重试合成一个 15 秒硬截断。 |
| TUNNEL_CONNECT_TIMEOUT 是 15 秒，TUNNEL_HANDSHAKE_TIMEOUT 是 device/mod.rs:86 的 5 秒 CDTunnel 参数交换期限。 | crates/ios-core/src/tunnel/mod.rs:9、crates/ios-core/src/device/mod.rs:86 | 15 秒初始化合同不得混称或替换现有 5 秒 CDTunnel handshake。 |
| ios-core 的 tunnel-kernel feature 为 tunnel + dep:tun-rs，tunnel-userspace 为 tunnel + dep:smoltcp。 | crates/ios-core/Cargo.toml:76-82 | 绑定 crate 必须独立声明 kernel 依赖 feature。 |
| ios-py/Cargo.toml:27 和 ios-ffi/Cargo.toml:19 目前只给 ios-core mdns,tunnel-userspace。 | 两个 manifest 的 dependency 行 | 公开的 kernel mode 不能只因 workspace feature union 看似存在。 |
| Python parse_tunnel_mode 接受 userspace 和 kernel，无效字符串返回包含两者的 ValueError。 | crates/ios-py/src/lib.rs:201-208 | 不得静默删除 kernel 字符串或改变无效 mode 合同。 |
| activate_tunnel 在未编入 kernel 时会返回 Unsupported，而编入时会创建 KernelTunDevice 并转发 packet。 | crates/ios-core/src/device/tunnel_activation.rs:120-145 | 首选补齐 feature；只有证据证明平台确实不可行时才讨论收窄合同。 |
| pyproject.toml 只设置 maturin 的 extension-module，host binding tests 需与 wheel 构建分开。 | crates/ios-py/pyproject.toml:37-42 | 不得把 workspace --all-features 当作 wheel 独立构建证据。 |
| release、python-wheels、publish-crates 当前的 needs 主要只有 check, build-and-test, validate-tag；python-sdist 只有 check, validate-tag；publish-pypi 只依赖两个 Python 产物 job。 | .github/workflows/ci.yml:201-460 | 至少接入 check、feature-check、msrv、build-and-test、validate-tag；Python 还要接 python-smoke。 |
| release 的 FFI 打包步骤对库文件使用 cp ... || true，可能静默漏包。 | .github/workflows/ci.yml:283-313 | 期望文件必须显式存在，缺失时 job 失败。 |
| wheel 当前只 build/upload，没有在各目标 runner 安装实际生成的 wheel。 | .github/workflows/ci.yml:324-390 | 上传前须对刚生成的 artifact 做 clean-venv import/error-contract smoke。 |
| XpcConnection 将分片状态放在 framer/per-stream buffer 中；recv_fresh_on_stream_inner 先读 24-byte XPC header 再读 body。 | crates/ios-core/src/xpc/rsd.rs:762-910 | 取消测试必须覆盖 header、body、stream 路由和内存预算。 |
| InstallCoordinationProxyClient::query 已在第一处 I/O 前同步把连接标为不可用，失败或取消需重连；完整非空响应后恢复可用。 | crates/ios-core/src/services/installcoordination.rs:170-263 | 这是 poison 语义参考；同步预标记或合适的 Drop guard 均可，不能依赖被取消 future 后续执行清理。 |
| 当前没有专门证明通用 XPC 取消会失步的复现结论。 | 本任务起点 | 先复现；复现失败时不能把静态风险写成已证实缺陷。 |

已由上一轮宿主检查得到的基线记录是：Windows、Rust 1.98 下 cargo test -p ios-core --all-features --locked --offline --quiet 为 947 passed, 1 ignored，cargo fmt --all -- --check 通过。这是基线证据，不是本轮未来修改后的验证，也不覆盖 Linux/macOS、Python wheel、FFI release 或真机。

任务书复核时另执行了 `cargo tree --locked --offline -p ios-py -e features --depth 2` 和对应 `-p ios-ffi` 命令，两者的直接依赖 feature 均为 `default`、`mdns`、`tunnel-userspace`，没有 `tunnel-kernel`。这是独立依赖解析证据，不是 kernel 的构建或运行成功证据。文中行号只用于定位基线；代码漂移时以文件内符号为准。

## 全局执行约束

1. 保持公开 Rust、Python、C API 兼容；需要增加内部 helper、私有状态或测试注入点时优先采用最小范围。不要把所有 service operation、长寿命 stream 或非幂等请求统一包进 15 秒。
2. 不因超时而自动重试非幂等操作。初始化失败可丢弃当前连接并按已有策略重新建 stream，但不能重放已经发送的业务请求。
3. 新增测试应验证具体行为：有限时间返回、预算不重置、正确 fallback、实际产物可导入、库文件不可漏包、取消后可复用或明确拒绝复用。不要堆与实现无关的快照或重复 happy path。
4. 若测试需要短时间，优先注入 Duration、使用 duplex/oneshot 控制读写阶段，或只在 test 配置中显式启用 tokio 的 time testing；不要用未经同步的毫秒 sleep 竞态证明成功。
5. 相同失败最多额外重试两次，且每次必须带来新证据。没有新证据、需要未授权动作或只能靠真机/凭据推进时，暂停该子项并继续其他独立子项。
6. 每项开始时记录验证时限：首次编译建议上限 30 分钟，增量测试套件 10 分钟，单个协议行为用例另设秒级看门狗；首次 release/wheel 构建建议上限 45 分钟。仍有明确编译进度时可说明理由调整一次；无输出且无进展时检查本次进程/日志，不能无限等待。以上是验证进程预算，不是产品超时合同；编译超时不能记成协议测试失败。单个复现假设调查建议上限 45 分钟，无新证据时提前结束；测试通过后不因形式要求重复完整套件。
7. 不提交、推送、打 tag、真实 publish、下载凭据、访问真实设备或修改 pair record。CI workflow 的模拟/静态验证可以改文件，但不得触发 release side effect。
8. 每个任务交付“改了什么、为什么、测试命令及结果、未运行项目、风险”的短证据；未知和 NOT_RUN 要原样保留。

## 任务总表与依赖顺序

| ID | 优先级 | 主题 | 依赖 | 主要允许修改 | 完成标志 |
|---|---:|---|---|---|---|
| T0 | 前置 | 预检与范围冻结 | 无 | 无产品修改；可记录报告 | 实际 HEAD、工作树和环境边界已记录 |
| T1 | P1 | H2/XPC/RSD 初始化期限 | T0 | ios-core XPC/device 测试和最小实现 | 本文列出的无界初始化有期限，且预算与 fallback 证据齐全 |
| T2A | P1 | CI 发布门禁有向图 | T0 | .github/workflows/ci.yml | release、crate、Python 发布链路至少依赖规定门禁 |
| T2B | P2 | release/wheel/sdist/FFI 实际产物 smoke | T2A；最终产物验证须包含 T3 | .github/workflows/ci.yml，必要时专用验证脚本 | 每个实际 artifact 在上传前被检查；未执行 publish |
| T3 | P1 | Python/FFI kernel feature wiring | T0 | 两个 Cargo manifest、针对性测试/CI | 独立 package 构建可见 tunnel-kernel，mode 合同不静默缩水 |
| T4 | P2 | XPC 取消后复用诊断与最小修复 | T1；与 T1 共享 XPC 文件时串行 | xpc、RSD fallback 和针对性测试 | 半头/半正文取消结论真实；复现才修，fallback 不误伤 |
| V | 收尾 | 集成验证与交付记录 | T1、T2A、T2B、T3、T4 | 必要的测试/报告，不新增方向 | 命令矩阵、diff、NOT_RUN 和风险模板完整 |

推荐顺序为 T0 → T1 → T4，T0 → T2A → T2B，T0 → T3，最后 V。T1 与 T4 都可能触碰 xpc/client.rs、h2_raw.rs、rsd.rs，不能在同一工作树中互相覆盖未验证的改动；先合并 T1 的 deadline 语义，再做取消结论。T2A 完成后才扩展 artifact smoke，避免 workflow 依赖与产物检查同时膨胀。

由单个 glm-5.3-flash 执行时，推荐直接采用 T0 → T1 → T3 → T2A → T2B → T4 → V。上面的依赖图不要求额外创建代理；若有效指令要求并行，应先分配不重叠的文件范围。T2A/T2B 统一修改 CI，T3 的 CI 验收需求交由该修改者集成。T4 改动影响最终产物时，V 只补跑受影响的构建/检查。

## T0：预检与范围冻结

### 目标与证据入口

确认执行环境、实际基线和工作树状态，不修改产品。入口是根 Cargo.toml、CONTRIBUTING.md、.github/workflows/ci.yml 以及本任务书列出的源码位置。

### 执行方式与决策条件

运行开场指令中的两条 git 命令，记录实际输出摘要。若工作树已经有用户修改，按路径区分并保留；若修改和本任务重叠，暂停该重叠子项，不能覆盖。确认 Cargo.toml workspace version 是否仍为 0.1.14，否则在最终报告中注明版本漂移。

### 允许修改与不做事项

T0 只读，不允许修改产品文件。不得 reset、clean、stash、checkout 或提交来“整理”基线。正常依赖构建按当前环境权限执行；安装系统工具、修改主机配置或访问凭据不属于本次预检。后续如需隔离实现，可按有效指令创建分支或 worktree，不得覆盖用户改动。

### 验收与交付证据

必须记录：实际 HEAD、git status --short、workspace version、Rust/Cargo 版本、可用 target/runner 信息，以及真机/外部发布均为 NOT_RUN 的声明。若预检失败但仍有独立只读证据可收集，说明失败并继续不依赖它的任务；若无法确认目标文件版本，停止改动。

## T1：给 H2/XPC/RSD 初始化加受控期限

### 目标

消除 XpcClient::connect_stream 与 open_rsd_proxy_framer 中此前可能永久挂起的初始化等待。15 秒只表示一次初始化尝试的共享预算；它不是把 5 秒 CDTunnel handshake 改成 15 秒，也不是所有 service operation 或长期 stream 的默认期限。

### 证据入口

- crates/ios-core/src/device/connected.rs:310-336：直接的 service path。
- crates/ios-core/src/xpc/client.rs:25-63：已有限期的 connect 与无期限的 connect_stream。
- crates/ios-core/src/xpc/h2_raw.rs:249-326：H2 写入、SETTINGS 等待、9-byte header/payload 读取。
- crates/ios-core/src/device/rsd_connect.rs:41-232：RSD 多阶段和重开 framer。
- crates/ios-core/src/tunnel/mod.rs:9、crates/ios-core/src/device/mod.rs:86：15 秒 TCP 连接和 5 秒 CDTunnel handshake 常量。

### 建议方案与决策条件

1. 先明确 deadline 起点。建议在 stream 已建立且开始 H2/XPC 初始化时创建 Instant deadline；H2 preface/SETTINGS/WINDOW_UPDATE、服务端 SETTINGS 和 XPC 初始化步骤共用同一 deadline。每个阶段只计算剩余时间，不能各自重新启动 15 秒。
2. 对 connect_xpc_service_with_metadata，增加内部 deadline-aware helper 或等价最小路径，使 H2Framer::connect 与 initialize_xpc_connection_on_framer 由同一个期限包住。保持返回的 XpcClient、metadata 和现有公开调用形状；超时应转为现有错误体系中可识别的 I/O/timeout 错误。
3. 审查 XpcClient::connect_stream 的所有调用者后再决定是否让原函数委托给带期限 helper。若公开函数语义不能安全改变，可只在 service path 调用内部带期限版本；若统一补期限，必须说明对现有 callers 的行为变化并添加回归测试。
4. 对 open_rsd_proxy_framer，至少给每一次此前无界的 H2Framer::connect 添加 15 秒的单次初始化上限，同时保留现有 TCP dial 预算。新 framer 超时后应被丢弃，不能继续向可能半读的 stream 写业务数据。
5. 不默认把 queued、legacy、passive 的全路径合计截成一个 15 秒 budget。若实现者选择总预算，必须先定义“queued 失败后 legacy、legacy 失败后 passive”的兼容规则，证明每次重新开 framer 的可达性和不会误杀已有 fallback；否则保持各现有阶段期限，只补无界 H2 初始化。
6. 不把 TUNNEL_HANDSHAKE_TIMEOUT 改为 15 秒，不改变既有 TCP connect 的已有预算，不给普通 call、文件传输、长寿命 stream_invoke 自动加 15 秒。

### 允许修改范围

允许修改 crates/ios-core/src/xpc/client.rs、h2_raw.rs、rsd.rs、device/connected.rs、device/rsd_connect.rs 及这些模块内的单元测试；需要共享的私有 deadline 类型可放在 ios-core 内。允许更新最小错误文本和 tracing stage 名称。不要修改与本初始化合同无关的服务实现。

### 不做事项

- 不对非幂等请求自动重试。
- 不把整个 device connect、所有 RSD fallback 或所有服务调用机械套一层 15 秒。
- 不通过 sleep 伪造超时，不把真实设备等待当作单元测试。
- 不声称每种 iOS 版本都已由真机证明；缺设备时记录 NOT_RUN。

### 精确验收测试矩阵

| 场景 | 受控输入 | 必须观察的行为 |
|---|---|---|
| H2 服务端不发 SETTINGS | duplex/脚本 stream 完成客户端写入后不提供 9-byte frame | 在注入的短期限内返回 timeout；测试不能永久挂起。 |
| H2 header 分片 | 服务端只发 1 至 8 个 header byte，随后由 oneshot 决定是否补齐 | 期限从初始化起只计算一次；不补齐时有限返回。 |
| H2 payload 分片 | 发完整 header 后只发部分 payload | 有限返回并丢弃当前连接；不把残片交给后续 service call。 |
| XPC 初始化停顿 | H2 SETTINGS 成功，XPC 初始化响应由 oneshot 延迟 | H2 已消耗的时间计入同一 budget；不能在 XPC 阶段再得到完整 15 秒。 |
| 正常 service 初始化 | 脚本 peer 按现有 preface/SETTINGS/XPC 顺序回应 | XpcClient 和 ResolvedServiceMetadata 正常返回，wire 顺序未变。 |
| RSD framer 停顿 | proxy TCP 已建立但 H2 SETTINGS 不来 | open_rsd_proxy_framer 在单次初始化期限内返回 None/现有失败语义，不永久挂起。 |
| queued→legacy | queued 成功/失败各一条脚本路径，legacy 使用新 framer | 现有 fallback 仍能到达；失败 framer 不被复用。 |
| legacy→passive | 分别覆盖现有会进入 passive 和会直接返回失败的分支 | 保持现有分支可达性；若复用已失同步的 framer 不安全，须用受控协议验证状态保留或重连方案，不能假定换 socket 就能保持 passive 语义。 |
| deadline 边界 | 使用可注入短 Duration 或 test clock，不用竞态 sleep | 每个 timeout 错误能定位阶段，且没有每阶段预算重置。 |
| 既有 TCP 行为 | TCP 连接失败/超时脚本 | 仍使用已有 TUNNEL_CONNECT_TIMEOUT 和错误语义，不重复包装成 CDTunnel handshake。 |

### T1 交付证据与停止条件

交付应包含：deadline 起点与剩余时间算法、受影响 callers 清单、RSD fallback 选择、测试名称/命令/结果、正常路径与 timeout 路径的错误摘要。改前测试应由外部看门狗明确观测“产品未自行按预算返回”，不能等待真实无限挂起；修复后同一场景应由产品期限先返回。若只有粗粒度 wall-clock 测试而无法证明预算不重置，先补可控测试。若平台或依赖缺失，记录 NOT_RUN 并保留已完成的 host 证据。

## T2A：修正发布质量门禁有向图

### 目标与证据入口

让 release、crate publish、Python wheel/sdist/PyPI 链路在产生外部可见产物前至少经过 check、feature-check、msrv、build-and-test、validate-tag；Python 产物还必须经过 python-smoke。入口为 .github/workflows/ci.yml:13-199 的质量 jobs 和 :201-460 的 release/publish jobs。

### 建议方案与决策条件

1. 用 GitHub Actions needs 形成清晰的有向图。可以让发布 job 直接列出所有必需 gate，也可以引入一个只聚合 gate 状态的 job，再让发布 job 依赖聚合 job；不要求所有 job 机械互相依赖或失去并行性。
2. release 与 publish-crates 至少要传递 check、feature-check、msrv、build-and-test、validate-tag。python-wheels 还要传递 python-smoke 以及上述核心 gate；python-sdist 至少要传递上述核心 gate，不能只依赖 check/tag。publish-pypi 要通过 python-wheels、python-sdist 间接继承全部 gate，并保持只在实际 artifact smoke 成功后运行。
3. 保留 validate-tag 只在 tag 触发时运行的语义；不要为了补依赖把普通 branch/PR 变成 publish。
4. 修正后的 YAML 要能由静态审查看出所有发布节点的上游质量门禁；不要只在说明文字中声称已经检查。

### 允许修改范围与不做事项

只允许修改 .github/workflows/ci.yml 中 job 依赖、条件和验证步骤所需的最小内容。不得增加真实 publish、tag、凭据读取或新的外部服务。不得以 workspace --all-features 代替 Python 独立构建，也不得删除现有跨平台 matrix。

### 精确验收矩阵

| 图节点 | 验收 |
|---|---|
| release | YAML 中直接或经 gate aggregator 依赖五项核心 gate；tag 条件保留。 |
| publish-crates | 依赖五项核心 gate；scripts/publish-crates.sh 仍是唯一 publish 命令，当前验证只做静态检查。 |
| python-wheels | 依赖五项核心 gate 加 python-smoke；每个 matrix target 的 build 和 smoke 结果再允许上传。 |
| python-sdist | 依赖核心 gate；sdist 生成前不跳过 msrv/feature/build 流程。 |
| publish-pypi | 仅依赖已完成 wheel/sdist 检查的 artifact jobs；本轮不触发该节点，不能指望正常 release tag 下的权限自动阻止发布。 |
| 普通 CI | check、feature-check、msrv、build-and-test、python-smoke 仍能按原触发条件运行，不因 needs 形成永远跳过。 |

### 交付证据与停止条件

交付包括修改后的 job graph 摘要、每个发布节点的实际 needs、YAML lint/解析或等价静态证据，以及未触发 publish 的证明。若 GitHub Actions 无法在本地完全模拟，标注 runner-only 部分 NOT_RUN；不得把 YAML 能解析误报成远端 job 已成功。

验证门禁需要至少一条负向证据：在本地 job graph 检查器的合成状态中，分别令 `feature-check`、`msrv`、`python-smoke` 为 failure/cancelled/skipped，确认受影响发布节点不具备执行条件。检查 `if: always()`、`continue-on-error` 或聚合 gate 是否绕过失败；不要只做 needs 字符串存在性断言。若使用 `always()`，聚合节点必须显式要求每个必需 gate 为 success。只修改检查器的输入状态，不制造真实失败 release 或推 tag。

## T2B：实际产物、FFI 包和 wheel/sdist smoke

### 目标与依赖

T2B 依赖 T2A，最终产物必须包含 T3 的 feature 修复，重点是“刚刚生成的那个 artifact 能用、完整且才可上传”。不要求本地真的发布；matrix runner 的跨平台结果由 CI 自己记录。

### release/FFI 方案

1. 在 release matrix 的每个 target 上，先完成 cargo build --release --package ios-cli --package ios-ffi --target ${{ matrix.target }}，再按 target/OS 明确检查 CLI、FFI 动态库、FFI 静态库和 header 的期望路径。
2. 删除 cp ... 2>/dev/null || true 这种静默成功。缺少必需文件时立即失败并打印 target、release directory 和期望文件名；如果某平台的文件名约定不同，显式列出该约定，不要用宽泛 glob 把漏包隐藏。
3. 打包后先解包到临时目录，检查 CLI 可执行文件、ios_rs.h、至少一个静态库和该平台应有的动态/导入库均存在且非零长度，再生成 checksum。不同 matrix target 可分别上传 artifact，不新增事务性 release 设计。
4. 对解包后的真实 FFI 包编译并链接一个无设备 C smoke。优先调用稳定的无设备边界：ios_free_string(NULL)、空输出参数的拒绝路径（例如 ios_list_devices(NULL, NULL) 返回非零）和一个不会连接设备的生命周期/空参数检查。不要向 IosTunMode 传非法整数；C 传 Rust repr(C) enum 的非法判别可能是未定义行为。
5. C smoke 不得调用 ios_start_tunnel、创建 TUN、读取 pair record 或访问真机；它只证明 header、符号、ABI 链接和安全的空参数错误合同。Unix 使用 runner 上可用的 C compiler 与 -I/-L/库路径，Windows 使用 runner 上可用的 MSVC/clang/gcc 等等价命令，并在报告中记实际命令。

必须执行链接后的 smoke 程序并检查退出码，只有编译成功不足以证明动态库能加载。动态加载路径只指向解包目录，不能误用 `target/release` 中的另一份库。MSVC 产物用兼容 ABI 的工具链，明确区分 `ios_ffi.dll.lib` 导入库与 `ios_ffi.lib` 静态库；Unix 明确区分 `.so`/`.dylib` 与 `.a`。本轮至少验证动态链接并运行，静态库检查文件完整性；若额外做静态链接需列出实际原生依赖，未做时不得声称静态链接已通过。CLI 解包后运行 `--version` 和 `--help`，禁止以 `list` 冒烟访问设备。使用合成缺文件目录验证包装检查会失败，清理仅限本轮记录且核验过绝对边界的临时目录。

### wheel/sdist 方案

1. python-wheels 每个 target 在 maturin build 后创建干净 venv，把 dist/*.whl 的实际文件安装进去，再从仓库外临时工作目录运行 Python。不能通过当前源码目录 import 来冒充 wheel 安装。
2. wheel smoke 至少检查 import ios_rs、callable(ios_rs.list_devices)、start_tunnel("no-such-device", mode="invalid") 抛 ValueError，且错误文字同时包含 userspace 与 kernel。这条检查不接设备、不启动 tunnel；mode 合法性的测试不等于 kernel TUN 已获得管理员验证。
3. 确认 wheel 名称、目标 tag、非零大小和安装来源属于当前 matrix build；避免 dist 中残留旧 wheel 被 glob 误选。每个 target 的 smoke 成功后才能 upload-artifact。
4. python-sdist 至少检查实际生成 tarball 非空、可列出 pyproject.toml、绑定 crate manifest 和源码所需文件；如果选择在 runner 安装 sdist，应把 build isolation 与网络依赖条件写入日志。不能把仅存在 tarball 当作可安装证明。
5. wheel/sdist smoke 不执行 PyPI 上传，不需要真实版本发布，也不要求设备。Linux CI 可按现有 workflow 使用 PYO3_PYTHON=/usr/bin/python3；Windows 命令不要照搬这个 Linux 路径。

### 允许修改与不做事项

允许修改 .github/workflows/ci.yml 中的打包、检查、upload 顺序，必要时增加短的 CI-only shell/Python/C smoke 内容。若新增脚本，必须是本任务直接需要的、无凭据和无设备副作用的小工具，并在交付中说明；不要借机重构发布系统。当前不运行 publish-pypi、publish-crates 或 GitHub release action。

### T2B 精确验收矩阵

| Artifact | 必做检查 | 上传前条件 |
|---|---|---|
| CLI archive | 解包；文件存在且非零；`--version`/`--help` 正常退出；checksum 生成 | package check 成功 |
| Unix FFI archive | .so/.dylib、.a、header 按 target 存在；真实 archive 上 C smoke 动态链接并执行 | 所有期望文件和 smoke 成功 |
| Windows FFI archive | DLL、导入/静态库和 header 按实际 MSVC 输出路径存在；C smoke 动态链接并执行 | 所有期望文件和 smoke 成功 |
| abi3 wheel | 当前 target wheel 安装到 clean venv；import、callable、invalid mode ValueError | install/import/error contract 成功 |
| sdist | 实际 tarball 非空，必需 manifest/source 可列出；若执行安装则安装成功 | archive inspection（及已声明的 install）成功 |
| Python publish | 只消费所有 wheel/sdist 检查成功的 artifact | 本轮不触发此节点，不以权限失败作为测试手段 |

### 交付证据与停止条件

交付应逐 target 列出 archive/wheel 名、检查过的文件、C/Python smoke 命令及结果。某 runner 在当前环境不可用时记录 NOT_RUN，不能上传一个未 smoke 的 artifact。若同一 packaging failure 重试两次仍无新证据，停止该 target，保留其他 target 的 host/静态结果。

## T3：兑现 Python 与 C FFI 的 kernel feature 合同

### 目标与证据入口

使独立构建的 ios-py 与 ios-ffi 显式启用 ios-core 的 tunnel-kernel，同时保留 userspace、mdns 和现有 Python extension-module 分工。入口：

- crates/ios-core/Cargo.toml:76-82 的 tunnel-kernel/tunnel-userspace。
- crates/ios-py/Cargo.toml:15-29，尤其 dependency feature 行。
- crates/ios-ffi/Cargo.toml:15-22，尤其 dependency feature 行。
- crates/ios-py/src/lib.rs:201-208 的 mode parser。
- crates/ios-core/src/device/tunnel_activation.rs:120-178 的 cfg 分支。

### 建议方案与决策条件

首选直接把 tunnel-kernel 加到两个 crate 对 ios-core 的显式 features，例如保持 mdns、tunnel-userspace 并追加 tunnel-kernel；不要为了这件事新建公开 capabilities API，也不要依赖 workspace --all-features 的 feature union。检查 Cargo feature resolver 后，以 package 独立命令证明依赖确实存在。

如果某目标平台的 tun-rs 确实无法编译，必须给出具体 runner、编译错误、crate/target 限制和公开合同影响，再决定是否按平台条件收窄；不得静默删掉 kernel mode、改成永远返回 Unsupported，或仅修改文档掩盖缺失。没有这样的证据时，继续首选完整 feature wiring。

### 允许修改范围与不做事项

允许修改两个 binding 的 Cargo manifest、必要的 lockfile（只有 Cargo 实际要求时）和针对性 host/CI 验证。允许补 mode parser 或 feature 的小测试，但不得要求真的建 TUN、root/admin、pairing 或真机。不得将 workspace 全 features 测试作为唯一证据；不得把 extension-module 混入普通 host 测试造成 PyO3 链接冲突。

### 精确验收矩阵

| 检查 | 命令/输入 | 通过条件 |
|---|---|---|
| ios-core feature subset | cargo check -p ios-core --no-default-features、CI 中的 classic/developer/ios17/management checks | 原有 feature subset 仍能编译 |
| Python 独立 host | cargo test -p ios-py --no-default-features，必要时设置当前 OS 可用的 Python 开发环境 | host 测试通过，未启用 packaging-only extension module |
| FFI 独立 host | cargo test -p ios-ffi 或等价 cargo check -p ios-ffi | FFI 自己解析到 kernel/userspace 依赖，不依赖 workspace all-features |
| feature 证据 | cargo tree -e features -p ios-py、cargo tree -e features -p ios-ffi，或等价 metadata 输出 | 输出能看到 ios-core 的 tunnel-kernel 与 tun-rs 路径 |
| Python mode contract | 现有 Rust unit test 或 wheel smoke：userspace、kernel 可解析；invalid mode ValueError 含两种合法值 | 合同未静默缩水 |
| wheel build | 当前 target 的 uvx maturin build --release --target ... --out ... | 实际 wheel 成功生成；extension-module 仍只由 wheel build 开启 |
| privileged behavior | 不创建 TUN、不调用 kernel activation | 记录为未做设备/权限验证，不作为 host 失败 |

### 交付证据与停止条件

交付包括两个 manifest 的 feature diff、独立 package feature 树、host test/check 和 wheel build 结果、平台限制及 NOT_RUN 项。若 tun-rs 在某目标阻塞，先保留其他平台证据并停止该平台的扩展，不以删除 feature 作为临时修复。

## T4：诊断 XPC 取消后复用与最小修复

### 目标与事实边界

这是尚未专门复现的静态风险，必须先证据化。重点是 h2_raw.rs:294-318 的 9-byte header/payload read_exact、rsd.rs:905-935 的 24-byte XPC header/body 读取、client.rs:66-128 的 call/stream_invoke，以及 RSD legacy timeout 后继续在同一 framer 上 passive fallback 的路径。

### 第一阶段：可控复现

用 tokio::io::duplex 或现有脚本 IO，使用 oneshot 明确控制 peer 已发送的 byte 数和何时继续。至少分别让客户端在以下位置被 timeout 或主动取消：

同步点必须证明客户端已经消费了目标字节，而不只是 peer 已写入缓冲区；可用包装 AsyncRead 的计数/通知观察消费进度。必须实际 drop 被测试的 future/stream 并释放其借用，不能仅丢弃指向仍存活 future 的引用或等待句柄。为每个复现设置外部看门狗，防止错误状态把测试进程永久挂住。

1. H2 frame header 已收到 1、4、8 byte 时。
2. H2 完整 header 后只收到部分 payload 时。
3. XPC 24-byte header 已收到部分 byte 时。
4. XPC body 已收到部分 byte 时。
5. XpcClient::call 已发送 request、等待 reply 时。
6. stream_invoke 已发送 request、收到部分 stream 消息时，以及 future 尚未首次 poll 时。

每个场景随后都要尝试一次受控后续操作，并检查 wire：是 parser 能继续且完整重组，还是连接立即拒绝复用并且没有第二个业务 request。未 poll 的 future 不应因为被 drop 就改变连接状态。

### 第二阶段：根据证据选择方案

确诊失步后，先评估把 header/payload/parser 进度持久化到连接对象，使取消后仍能正确恢复解析；只有实现和测试证明状态完整保留，才能承诺复用。不能只修 H2 半帧而忽略上层已经消费的 XPC 头/正文阶段。必须保持：

- msgid 与 stream id 路由不串线；
- 每 stream 和连接级 buffer 上限继续生效；
- 正常完整消息、EOF、协议错误的既有行为保持；
- stream drop、重连和已缓存 pending message 的清理有明确规则。

若持久化方案超出最小修复范围，可选择与 InstallCoordinationProxyClient::query 类似的 poison 合同：进入可能失同步的 I/O 前同步标记连接不可用；取消后后续入口必须检查该状态并明确要求重连。可采用预标记/成功后恢复或同步 Drop guard，不规定必须使用某种机制。恢复点应是完整协议操作边界；完整业务错误响应、读到半包的错误和已污染连接应区别处理，并测试实际选择。stream 已 yield 一个完整消息且尚未开始下一次读取时也需明确可复用性。实现不能依赖“future 被 drop 后再执行清理语句”，不能误伤从未 poll 的 future，且不得通过 `send`、`recv_any` 或 pending message 快路径绕过 poisoned 状态。

无论选择哪条路，都要评估 call 的发送阶段取消和 stream_invoke 返回 stream 的 drop；只修复这些共享 primitive 必须覆盖的路径，不扩展重构所有服务。若第一阶段完整复现没有失步，记录“静态风险未证实”，保留能证明行为的测试，不做无证据的全局 poison。

### RSD fallback 特别约束

当前 legacy bootstrap 超时以及部分错误分支会在同一 framer 上尝试 passive；legacy handshake 超时分支则直接返回失败。先按现有 match 分支建立测试表，不为追求覆盖擅自增加 fallback。若采用 poison，评估保留同步状态或重新建连的协议条件，用受控 peer 证明 passive 服务目录仍可得到；不能假定新 socket 必然提供相同被动消息，也不能继续使用已失步连接。queued→legacy 已经重新开 framer 的路径也要证明没有复用已取消 stream。

### 允许修改范围与不做事项

允许修改 xpc/h2_raw.rs、xpc/rsd.rs、xpc/client.rs、device/rsd_connect.rs 及相应服务/测试中直接需要的 poison/fallback glue。不得改造所有 service API、引入全新错误体系、增加业务重试、扩大 buffer 或删除多 stream 路由。不得以真机日志取代可控半包测试。

### 精确验收测试矩阵

| 场景 | 断言 |
|---|---|
| H2 半 header 取消 | 按选定 contract：持久化方案可继续完整 frame；poison 方案后续调用立即返回 unusable 且不写第二个 request。 |
| H2 半 payload 取消 | 同上；不得把残余 payload 与下一 frame header 拼接错误。 |
| XPC 半 header 取消 | 24-byte header 不丢 byte、不跨 stream 拼接；失败时明确需要 reconnect。 |
| XPC 半 body 取消 | body 长度、decoder、buffer budget 不被破坏；下一次操作符合选定复用合同。 |
| call send/receive cancel | 已发送非幂等 request 不被自动重放；取消后状态明确。 |
| stream_invoke drop | 未 poll 不误 poison；已开始 receive 的 drop 按选定合同处理；正常 EOF/error 保持可观察。 |
| RSD legacy→passive | 对现有可进入 passive 的分支，状态保留/重连策略须经受控 peer 证明；不能复用失步连接或新增未要求的 fallback。 |
| msgid/multi-stream | server-client/client-server 与不同 msgid 的缓存和取回顺序保持；不突破现有 pending memory cap。 |
| 正常完成 | 完整 H2/XPC response 后连接仍按原 API 可用，未引入每次 call 的固定 15 秒上限。 |
| EOF/协议错误/重连 | EOF 和协议错误返回既有错误类型或清晰转换；重连后首个请求可正常开始。 |

### T4 交付证据与停止条件

交付必须写明：哪一个受控半包场景复现/未复现、取消点、改前/改后的同一测试结果、选定持久化或 poison 的理由、未 poll 处理、RSD fallback 影响、测试输出和未做真机项。同一假设无新证据时遵循有限重试和调查时限，报告未证实结论；不同取消位置可以独立调查，不能把前三个测试尝试失败当作覆盖全部风险。仅“没复现”不能算风险证伪；只有消费进度和后续行为证据充分、相关矩阵通过，才可结论为无需产品修改。

## V：整体验证命令与交付要求

### 推荐验证顺序

执行代理在代码和 workflow 变更完成后，按环境可行性运行以下命令；命令是验收矩阵，不要求当前任务书作者运行。--offline 只有依赖已在缓存时使用，不能因离线缺包伪造失败结论。

~~~powershell
git status --short
git rev-parse HEAD
cargo fmt --all -- --check
~~~

宿主 Rust 分层检查（遵循 CONTRIBUTING.md 与 CI 对 PyO3 的拆分）：

~~~text
cargo test --workspace --exclude ios-core --exclude ios-py
cargo test -p ios-core --all-features --locked
cargo test -p ios-py --no-default-features
cargo test -p ios-ffi
cargo clippy --workspace --all-targets --all-features -- -D warnings
~~~

feature/MSRV 检查应与现有 CI 一致：

~~~text
cargo check -p ios-core --no-default-features
cargo check -p ios-core --features classic
cargo check -p ios-core --features developer
cargo check -p ios-core --features ios17
cargo check -p ios-core --features management
cargo check --workspace --exclude ios-py        # MSRV runner 使用 Rust 1.80
~~~

上面的最后一条只有在实际 `rustc --version` 为 1.80 的环境中才构成 MSRV 证据；若已安装该工具链，可明确运行 `cargo +1.80 check --workspace --exclude ios-py --locked`。在 Rust 1.98 上运行普通 check 不得记为 MSRV 通过。两组测试中的重复覆盖只执行一次：workspace 非 core/py 测试用于集成，单独 `-p ios-ffi` 用于检查绑定独立 feature 构建；成功后仅在新增变更、失败或未解决问题需要时重跑。若用 paused time 或私有短预算测试需要改 manifest，只允许加入直接需要的测试依赖/feature，并保持生产 feature 合同。

Python host 与 wheel 使用分离命令：Linux CI 可设置 PYO3_PYTHON=/usr/bin/python3 后运行 cargo test -p ios-py --no-default-features；wheel 在 crates/ios-py 用 uvx maturin build --release --out ../../dist 或对应 matrix target 构建，再安装刚生成的 wheel。Windows 不要使用 /usr/bin/python3 路径。

发布 workflow 的本地可做验证包括 YAML 解析、job needs 静态检查、target 文件路径检查脚本审阅和 smoke 脚本审阅；不要执行 scripts/publish-crates.sh、PyPI action、GitHub release action 或真实 tag。aarch64 Linux、macOS、Windows runner 的实际编译/安装和 C link 若本环境没有，统一标 NOT_RUN，由对应 CI runner 产生证据。

### 最终交付模板

执行代理最终请按下面格式返回给主代理，保持短而有证据：

~~~text
结论：T1/T2A/T2B/T3/T4 各自 DONE、PARTIAL、NOT_REPRODUCED 或 BLOCKED。
- T4 的 DONE 必须说明“已修复并验证”或“已用决定性证据证伪，无需改产品”。
- NOT_REPRODUCED 表示尚未取得决定性证据，不能等同于修复或证伪。

变更：
- <文件路径>: <行为变化与原因>

关键证据：
- 基线 HEAD / 工作树：<值>
- <命令或 CI job>: <通过结果或失败摘要>
- <关键协议/feature/产物证据>: <摘要>

风险与不确定性：
- <未复现、平台限制、runner-only 或行为变化>
- <真机、管理员权限、发布、凭据均为 NOT_RUN 的项目>

建议动作：
- <下一步最小动作；只写仍在目标内的动作>
~~~

### 全局完成条件

仅当 T1 的期限修复、T2A 的门禁图、T2B 的产物检查实现、T3 的独立 feature 和 T4 的取消结论都有对应证据，才能报告“实现与本机可行验证完成”。该表述可以附明确的跨平台 NOT_RUN 清单，但不能等同于“跨平台产物验证全部通过”或“可发布”。实际发布准备就绪需要所有必需平台/产物 gate 的真实成功结果。子项缺乏决定性证据时报告 PARTIAL、NOT_REPRODUCED 或 BLOCKED；不得用基线 947 passed, 1 ignored 冒充新改动已验证。
