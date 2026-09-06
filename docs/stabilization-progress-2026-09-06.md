# rust-ios-device 稳定性任务执行进度记录

> 执行代理:glm-5.3-flash
>
> 记录时间:2026-09-06
>
> 基线 HEAD:32f3867cab4d937a4e8fe962227fe1de06b52081(workspace 0.1.14,与任务书一致)
>
> 配套任务书:docs/stabilization-tasks-for-glm-5.3-flash-2026-09-06.md
>
> 执行顺序:T0 → T1 → T3 → T2A → T2B → T4(进行中,见"未解决问题")

## 结论总览

| 任务 | 状态 | 摘要 |
|---|---|---|
| T0 预检 | DONE | HEAD 与基线一致;工作树仅含任务书;版本 0.1.14;rustc 1.98.0(msvc),1.80 工具链可用 |
| T1 初始化期限 | DONE | H2/XPC/RSD 初始化全部有共享 deadline;7 个新测试通过 |
| T3 kernel feature | DONE | ios-py/ios-ffi 独立解析到 tunnel-kernel;wheel 构建并冒烟通过 |
| T2A 发布门禁图 | DONE | 4 个发布节点接入 5 项核心门禁;静态检查器含负向证据 |
| T2B 产物 smoke | DONE(CI 侧) | workflow 打包校验 + C/Python smoke 就绪;本机验证 Windows 腿 |
| T4 取消复用诊断 | PARTIAL | 失步已复现并修复(H2 半帧恢复 + XPC body 中毒);client 级 1 个测试挂起待查 |

## T0:预检(只读)

- `git rev-parse HEAD` = 32f3867…(与任务书基线一致)。
- `git status --short`:仅 untracked 任务书本身,无用户产品改动。
- Cargo.toml workspace version = 0.1.14,rust-version = 1.80。
- 环境:Windows 10.0.26200,rustc/cargo 1.98.0(x86_64-pc-windows-msvc 默认),rustup 另有 1.80 工具链。Python 环境经 uv 管理(`.venv`,CPython 3.11.10)。

## T1:H2/XPC/RSD 初始化受控期限

**改了什么**

- `crates/ios-core/src/xpc/h2_raw.rs`:新增 `H2Framer::connect_with_deadline(stream, deadline)`,client preface/SETTINGS/WINDOW_UPDATE 写入与等待 server SETTINGS 共享同一个 `tokio::time::Instant` deadline,各阶段只取剩余时间;超时错误可定位阶段(写入阶段 / 等待 SETTINGS)。原 `connect()` 保持不变(调用方 rsd_handshake 已有外层 15s 包裹)。
- `crates/ios-core/src/xpc/client.rs`:`XpcClient::connect_stream` 委托给新增 `pub(crate) connect_stream_with_budget(stream, budget)`,H2 握手 + XPC bootstrap 共享一个 15s 预算(`TUNNEL_CONNECT_TIMEOUT`),XPC 阶段只拿 H2 未消耗的剩余时间;超时丢弃该次连接,不把半读状态交给业务调用。公开签名不变。
- `crates/ios-core/src/device/rsd_connect.rs`:`open_rsd_proxy_framer` 委托给新增 `open_rsd_proxy_framer_with_budget`(每次开 framer 一个 15s 单次初始化上限);queued(3s+4s)/legacy(3s+3s)/passive(2s) 既有阶段预算与 fallback 结构未动,未做全路径 15s 硬截断。
- `crates/ios-core/Cargo.toml`:新增 `[dev-dependencies] tokio test-util`(仅测试用 paused clock;生产 feature 集不变)。

**测试与结果**(`cargo test -p ios-core --all-features --offline`)

- `connect_stream_with_budget_times_out_when_server_withholds_settings`:server 收完握手不回 SETTINGS → 注入 150ms 预算内返回超时,错误可定位阶段。
- `connect_stream_with_budget_times_out_on_partial_h2_header`:server 只发 4/9 字节 header → 有限返回。
- `connect_stream_with_budget_shares_one_deadline_across_h2_and_xpc_stages`(`start_paused`):H2 用掉 250ms 后 XPC 停顿 → 总耗时=完整共享预算(500ms 虚拟时间),证明 XPC 阶段没有重置预算;错误信息定位到 XPC 阶段。
- `unbudgeted_h2_connect_does_not_return_when_server_stalls`:对照证明旧的无预算 `H2Framer::connect` 在同场景下不返回(看门狗 300ms 触发),即"改前"行为证据。
- `rsd_proxy_queued_failure_falls_back_to_legacy_with_new_framer`:受控 proxy 脚本(真实 TcpListener)驱动 queued 失败 → legacy;断言 accept==2(失败 framer 不被复用,legacy 用新连接)。
- `rsd_proxy_passive_fallback_reuses_framer_after_legacy_handshake_failure`:legacy 握手失败 → passive 在同一 framer 上恢复;断言 accept==2。
- `rsd_proxy_returns_none_when_proxy_port_is_closed`:代理端口关闭 → 快速返回 None。
- 全量:`cargo test -p ios-core --all-features` = **954 passed, 0 failed, 1 ignored**(基线 947 + 新增 7,无回归);`cargo fmt --check` 通过。

## T3:Python/FFI 的 kernel feature 合同

**改了什么**

- `crates/ios-py/Cargo.toml`、`crates/ios-ffi/Cargo.toml`:对 ios-core 的显式 features 由 `["mdns", "tunnel-userspace"]` → `["mdns", "tunnel-kernel", "tunnel-userspace"]`。

**证据**

- `cargo tree -e features -p ios-py` / `-p ios-ffi`:两者均出现 `ios-core feature "tunnel-kernel"` → `tunnel`,且 `tun-rs v2.5.1` 特性路径可见(改前任务书已记录两者均无 tunnel-kernel)。
- `cargo check -p ios-ffi --locked --offline` 通过(Windows 上 tun-rs 可编译,无平台收窄必要)。
- `cargo test -p ios-py --no-default-features --locked --offline` = 6 passed(含 `tunnel_mode_rejects_unknown_values_with_value_error`:mode 合同未缩水)。需 `PYO3_PYTHON` 指向 uv venv、PATH 含 uv 管理 Python 的 DLL 目录(见"环境注意")。
- wheel 构建:`uvx maturin build --release --out ../../dist` → `rust_ios_device_tunnel-0.1.14-cp39-abi3-win_amd64.whl`(4.6MB);干净 venv 安装后从仓库外目录运行:`import ios_rs` ✓、`callable(ios_rs.list_devices)` ✓、`start_tunnel("no-such-device", mode="invalid")` 抛 ValueError 且同时含 "userspace"/"kernel" ✓。
- sdist 构建:`uvx maturin sdist` → tarball 625KB;`tar -tzf` 可列出 pyproject.toml / Cargo.toml / Cargo.lock / crates/ios-core/src/lib.rs(maturin 会打包 path dependency)。
- 未做(按合同):真机 kernel TUN 激活、管理员权限验证 → NOT_RUN。

## T2A:发布质量门禁有向图

**改了什么**(`.github/workflows/ci.yml` 的 needs)

- release: `[check, feature-check, msrv, build-and-test, validate-tag]`(原缺 feature-check、msrv)
- publish-crates: 同上五项
- python-wheels: `[check, feature-check, msrv, build-and-test, python-smoke, validate-tag]`(原缺三项)
- python-sdist: `[check, feature-check, msrv, build-and-test, validate-tag]`(原仅 check+validate-tag)
- publish-pypi: 保持 `[python-wheels, python-sdist]`(传递继承全部门禁)
- 所有节点保留 `if: startsWith(github.ref, 'refs/tags/v')`;无 `if: always()`、无 `continue-on-error`。

**证据**

- 新增 `scripts/check-release-gates.py`(纯静态,不触发任何 run):对修复后 workflow **PASS**,并输出负向合成结论——把 check/feature-check/msrv/build-and-test/validate-tag/python-smoke 分别置为 failure/cancelled/skipped 时,所有传递依赖该门禁的发布节点均不具备执行条件。
- 对基线 workflow(git show HEAD)运行同一检查器 **FAIL**,逐条列出缺失门禁(release/publish-crates 缺 feature-check+msrv;python-sdist 另缺 build-and-test;python-wheels 另缺 python-smoke)——改前/改后对照证据。

## T2B:实际产物 / FFI 包 / wheel/sdist smoke

**改了什么**

- release job:删除 `cp ... || true` 静默漏包;每 target 显式断言 CLI、动态库、静态库、header 存在且非零(Windows 区分 `ios_ffi.dll.lib` 导入库与 `ios_ffi.lib` 静态库);打包后解包到临时目录验证;CLI 解包后运行 `--version`/`--help`(禁用 list 访问设备);checksum 生成。
- FFI C smoke(无设备、无 TUN、无 pair record):调用 `ios_free_string(NULL)`、断言 `ios_list_devices(NULL, NULL)` 返回非零、`ios_device_close(NULL)`/`ios_tunnel_close(NULL)` 为 no-op。Unix 用 cc/gcc/clang + `$ORIGIN` rpath(macOS 用 `install_name_tool -id @loader_path/...` 重写后链接),Windows 用 vswhere→vcvars64→cl,clang 兜底;**必须运行 smoke 可执行文件**并检查退出码;动态加载只指向解包目录。静态库做 `ar t` 完整性检查(Unix)。
- python-wheels:构建后清理并校验恰好一个非零 wheel → 干净 venv 安装 → 从临时目录(仓库外)运行 import/callable/invalid-mode-ValueError 断言 → 全部通过才 upload-artifact。
- python-sdist:构建后检查 tarball 非零、可列出 pyproject.toml/Cargo.toml/Cargo.lock/src/lib.rs,再 upload。
- `crates/ios-cli/src/main.rs`:clap `#[command]` 增加 `version`(此前 `ios --version` 不存在,T2B 验收矩阵要求 `--version`/`--help` 正常退出;最小一行改动)。

**本机验证(Windows 腿)**

- 本地 release 构建 ios-cli+ios-ffi 后,按 workflow 同样逻辑:4 个期望文件存在且非零 ✓;打包→解包→动态链接 smoke 运行通过,`ldd` 证实加载的是解包目录的 `ios_ffi.dll`(非 target/release)✓;`ios --version` 输出 `ios 0.1.14` ✓,`--help` 退出 0 ✓。
- 负向证据:合成缺文件目录时存在性检查按预期失败 ✓。
- 本机无 MSVC VC 工具集(vswhere 找到 BuildTools 但缺 SKU)、无 clang/gcc,故用 rustc+MSVC 导入库做了等价动态加载验证;**C 编译 smoke 的 MSVC/clang 腿留待 CI runner 产生证据**。

## T4:XPC 取消后复用诊断与最小修复(PARTIAL)

**第一阶段:可控复现(已完成,失步证据确凿)**

用 CountingStream(计数 AsyncRead/AsyncWrite,oneshot 通知精确消费点)+ 受控 duplex peer,在"客户端已消费目标字节"的前提下 drop 被测 future,再观察后续 wire 行为。改前(基线代码)结果:

1. `cancelled_read_mid_frame_header_resumes_without_desync`(半 header,1/4/8 字节):**FAILED** — 恢复读取把 payload 字节当 header 解析:`Protocol("frame payload 10855845 exceeds max frame size 16384")`(10855845=0xA5A5A5,恰为丢失半帧后的 payload 内容)→ 确认失步。
2. `cancelled_read_mid_frame_payload_resumes_without_desync`:**FAILED** — 同类失步。
3. `cancelled_write_mid_frame_is_completed_by_the_next_write`(小容量 duplex 强制半写入):**FAILED** — server 看门狗 10s 超时:未写完的帧字节随 future 丢失,server 永久等待,后续写入拼接进残帧。
4. XPC 半 header(缓冲层):缓冲字节本身不丢失(改前亦可恢复)——已作为回归测试保留。
5. call 取消(请求已发、等待回复):干净点,连接仍同步;晚到回复被 msgid 机制缓存。
6. stream_invoke:未 poll 的 drop 不影响连接;消息间 drop 为干净点。

**结论:静态风险被证实**——H2 半帧(header/payload)读取消与半帧写取消都会造成不可自愈的失步;XPC 层还有第二个窗口:24-byte header 已消费、body 未读完时取消,缓冲区剩余 body 字节会被下一条消息当 header 解析。

**第二阶段:修复(按任务书"持久化 + 边界 poison"混合方案)**

- `h2_raw.rs` 读路径:`read_raw_frame` 重写为可恢复——`partial_header`/`header_filled`/`pending_frame_meta`/`payload_buf`/`payload_filled` 持久在 framer 上,每次 await 前更新;取消后下一次读取从断点继续,不丢已消费字节、不重置预算。EOF 语义保持 UnexpectedEof。
- `h2_raw.rs` 写路径:所有 socket 写(ACK/WINDOW_UPDATE/HEADERS/DATA)统一走 `write_all_tracked`;`PendingWriteGuard` 在"取消且已写入部分字节"时把未写余量捕获到 `pending_write`,下一次写或读先补完该帧(`finish_pending_write`),server 端永不看到残帧。
- `rsd.rs` XPC 层:新增统一 `read_raw_xpc_message(framer, stream_id)`;header 消费后、body 读完前置 `message_read_in_progress` 标记(framer 字段),body 完成后清除;若 body 读取被取消,标记残留 → **所有后续入口拒绝**:`recv*`(含 pending-message 快路径之前)、`recv_any_stream`、`send_with_flags`,错误信息明确"连接已失帧对齐,必须重连"。未 poll 的 future、消息边界取消、零消费取消均不触发 poison。
- 修复后(通过):framer 级 3 项(半 header 1/4/8、半 payload、半写恢复)✓;XPC 级 2 项(body 取消 → 后续 recv/send 全部拒绝且 wire 上无第二请求、header 部分缓冲恢复)✓;`never_polled_stream_invoke_drop_is_inert` ✓;`stream_invoke_drop_between_messages_keeps_connection_usable` ✓。

**未解决问题(挂起测试)**

- `cancelled_call_is_not_replayed_and_connection_stays_usable`(`crates/ios-core/src/xpc/client.rs`)单独运行即挂起。设计流程:call #1 在 server 读到请求、尚未回复时取消 → server 补发迟到回复 → call #2 正常并断言 wire 上无重放。挂起位置未定位(主流程的 oneshot 等待点均无超时保护,server 脚本或某次 recv 可能未按预期推进)。已尝试:单测隔离运行确认可复现挂起;尚未加探针/超时定位根因。
- 影响评估:该测试是"改后回归验证"的一部分;**修复本身的核心断言**(半帧恢复、poison 拒绝、无第二请求)已由其他 7 项测试覆盖并通过。挂起根因可能是测试脚本自身(如 server 脚本与 cancel 时序的竞态)而非产品代码,但未证实。
- 全量回归与 clippy/fmt 在 T4 改动后**尚未重跑**;T4 相关文件未经过最终格式化检查。

## 全局验证状态(V 部分完成)

- T4 改动前全量:`cargo test -p ios-core --all-features` 954 passed / 1 ignored;`cargo fmt --all -- --check` ✓;`cargo clippy --workspace --all-targets --all-features -- -D warnings` ✓(exit 0)。
- T4 改动后:仅运行了取消相关测试子集;**全量测试/fmt/clippy 待重跑**。
- 未运行:MSRV(`cargo +1.80 check --workspace --exclude ios-py --locked`)、`cargo test --workspace --exclude ios-core --exclude ios-py`(T4 后)、真实 tag 下的 release/publish 链路、aarch64-linux/macOS runner 产物、PyPI/crates 发布——全部 NOT_RUN。

## 环境注意(本机复现要点)

- Python:uv venv `.venv`(CPython 3.11.10,已加入 .gitignore);PyO3 宿主构建需 `PYO3_PYTHON=<repo>/.venv/Scripts/python.exe`,运行 ios-py 测试二进制还需把 `%APPDATA%\uv\python\cpython-3.11.10-windows-x86_64-none` 加入 PATH(否则 STATUS_DLL_NOT_FOUND)。
- maturin 在后台 shell 中运行需显式 `PATH="$HOME/.cargo/bin:$PATH"`。

## NOT_RUN 清单

- 真机/iOS 设备验证、管理员权限 TUN 激活、pair record 相关操作。
- GitHub Actions 真实 runner 执行(release、python-wheels/sdist/publish、MSRV、aarch64-linux/macOS 产物、C smoke 的 MSVC/clang 编译腿)。
- 真实发布:publish-pypi、publish-crates、GitHub release、tag 推送。
- MSRV 1.80 编译验证(本机有 1.80 工具链但未运行)。

## 建议下一步

1. 定位 `cancelled_call_is_not_replayed_and_connection_stays_usable` 挂起根因(优先给 oneshot 等待点与 `client.recv()` 加超时,再用 eprintln/tracing 探针确认卡点),确认是测试脚本竞态还是产品缺陷。
2. T4 收尾后重跑全量 `cargo test -p ios-core --all-features`、`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`。
3. 运行 MSRV 检查与 workspace 非 core/py 测试,补齐 V 的命令矩阵。
4. 在正常 CI(用户授权的分支推送)上观察 release/wheel job 的 runner 侧证据(尤其 C smoke 的 MSVC 腿与 upload-artifact 前置条件)。
