# CLI 真机问题诊断与回归（2026-10-29）

本轮使用本地工作区和受信任的 D2（iPhone XR，iOS 18.7.10 / 22H374），
只复查上一轮的 16 个候选入口。没有重新验收全部 291 个入口。
分支 `codex/feature/windows-return-20261027`，HEAD
`ef5fd8cd6cb1c529a001d60bcef8d3e0923a21d9`；源码仍有未提交改动。

最终受测二进制 SHA256：
`2159d158275f9cf6b152149c121d77698825407ae687e1069c84786650e74f90`。
外部归档轮次为 `20261029-cli-diagnosis`；每次调用的 argv、原始输出、
PID、时间、退出和清理记录在其 `private/attempts/` 下。
`bin/verified-final.json` 将最终二进制绑定到源码哈希。

## 本轮结论

| 入口 | 结论与实际验收范围 | 原始 attempt |
| --- | --- | --- |
| forward --once | 已修复；真实 QueryType 返回 Lockdown Type，双方 EOF 后自然 exit0 | 024（旧）、086（新） |
| dproxy service heartbeat | 已修复；真实 Marco/Polo，双向完整 plist，客户端关闭后自然 exit0 | 025（旧）、087（新） |
| instruments sysmon-process | 已修复；重新观察 SpringBoard PID 后得到对应真实进程样本，count1 自然结束 | 088、089 |
| instruments sysmon-threshold | 已修复；threshold0/count1 得到真实样本，保留 u64 哨兵值 | 090 |
| viewer web | 已修复；GET /frame 返回可解码的 828×1792 真机 PNG；POST /stop 后自然 exit0 | 091 |
| viewer vnc | 已修复；真实 RFB 握手及 5,935,104 字节 framebuffer；本轮专属 Ctrl-C 后 exit0 | 093、094 |
| ddi cryptex nonce | 已修复；48 字节 nonce 与上游解包结果一致，两次 Rust 均自然成功 | 095–097 |
| os-trace archive | 已修复提前断链；完整归档仍未通过，见下文的输出上限 | 106、108、118、120、123、125 |
| os-trace collect | 同样修复提前断链；有界请求未产生完整目录，不计 PASS | 107、109 |
| tunnel serve | 原证据缺口已补齐；策略仅授权 D2，list/stop/空列表/shutdown 及监听关闭均有原始记录 | 111–115 |
| tunnel start | 原证据缺口已补齐；userspace 模式，独立 RSD 客户端确认 D2 和 59 项服务；退出与端口关闭已核实 | 116、117 |
| springboard wallpaper-home-screen | 当前设备共同限制；相同 USB 请求在 Rust 和 pmd 中均关闭连接；未取得完整壁纸 | 032、072、104 |
| webinspector opened-tabs | 外部前置不足；pmd 明确报告 Web Inspector 未启用，未修改设置 | 035、105 |
| debug | 外部前置不足；当前 59 服务目录没有 remote.debugproxy；未执行 LLDB、attach 或 launch | 071、103 |
| btlogger capture | 仍缺正向素材；3 秒自然结束但为 0 条 HCI，仅验证空捕获与收尾 | 102 |
| companion listen | 仍缺正向素材；配对列表为空，3 秒自然超时报错，没有手表事件 | 098、101 |

七个入口完成修复后的业务验收；两个 tunnel 入口补齐生命周期证据。
两个 OS trace 入口仅关闭了原连接生命周期缺陷，完整归档没有获验收。
三个入口受当前设备服务/设置限制，两个入口仍需真实事件素材。
上述分类不构成全 CLI 通过率。先前 apps icons、arbitration version、
MobileGestalt CoreDevice fallback 的 UPSTREAM_MATCH 结论没有被改列新 FAIL。

## 修复内容

- Forward 在复制业务数据之前优先取得 usbmux 的设备端口连接结果。
  自有 userspace 代理增加可选的连接确认字节，避免把本地 TCP 建连当成远端成功。
  普通 go-ios 格式 preamble 保持兼容；业务写入不重放；已建立连接的合法空 EOF 仍可成功。
  旧 USB 比较器与旧 Rust 默认路线不同，未把它们冒充同路线对照。
- Dproxy 只在客户端已完整关闭、远端没有残帧时接受随后发生的 TLS UnexpectedEof。
  未经客户端关闭的异常 EOF、截断长度头或截断帧体继续报错。
- Sysmon 的真实进程 archive 含有 u64::MAX。旧有符号整数解码拒绝整批样本，
  随后被静默跳过。现在保留完整无符号值，并传播畸形 archive、设备拒绝和属性查询错误。
- Viewer PNG 使用与独立截图相同的服务可用性 fallback 判断；DTX fallback
  使用原 NativeScope 并等待清理。PNG 具有独立的编码上限，解码像素仍限 12 MiB；
  HEVC access-unit 上限不变。旧实测约 3.15 MB PNG 及合成大 PNG 回归说明原 2 MiB 上限过小；
  最终真机帧为 37,197 字节，没有用合成图替代真机证据。
- Cryptex 接受 iOS 18 仅含 argv 的 get-nonce 成功响应，继续校验 nonce 包装与显式设备错误。
- OS trace 的连接函数同时返回并保留设备句柄，避免返回客户端时先销毁 userspace tunnel。

## OS trace 的剩余边界

原 ConnectionAborted 已消失。保持最近 60 秒、age1d 的请求范围，1 MiB 请求现在到达
真实数据传输并触发本地字节上限。独立诊断器保留设备句柄，确认原始错误为
`OS trace archive exceeds max size 1048576 bytes`；16 MiB/45 秒探测也明确触发
`OS trace archive exceeds max size 16777216 bytes`。
这些诊断器在打印错误后 exit0，不算归档成功。

Pmd 同样发送 SizeLimit；独立写入器在 8 MiB 上限拒绝其继续返回的数据。
因此没有移除本地上限来制造 PASS，也没有将超时、部分 tar 或清理后的暂存文件当成归档。
完整业务验收需要能容纳实际完整返回的输出预算，或更小的受控日志素材。

## 验证与收尾

串行 Cargo：444 项核心测试、95 项 CLI 测试、10 项 fetchsymbols 集成测试通过；
3 项 CLI 测试按原规则忽略。`git diff --check` 通过。
原 companion/fetchsymbols/h2_raw 四个交接文件哈希完全保留；新二进制再次返回
空 companion 列表、71 个唯一 symbols 路径及恰好 1024 字节 dyld 探测结果。

所有本轮服务进程均已退出；manager、standalone、Web、VNC、forward 的监听关闭已核对。
既有 2026-10-20 启动的 ios PID36976 未触碰。没有新增信任/配对、DDI 挂载、
系统路由、kernel tunnel、HID、应用运行、设置或设备文件写入。
两份本轮主机策略/token 文件因自动审批的策略拦截无法删除，已改为零授权策略与失效标记，
留在 private；没有活动凭据或服务器。原始图像、日志、应用/进程数据、凭据均不入 Git。

早期参数错误、诊断器构建失败、C 盘满导致的编译失败和显式强杀均保留。
`bin/final.json` 是已否决的过渡记录，不能用作最终二进制证明；最终只认
`bin/verified-final.json` 和 attempt084。未提交或 push。
