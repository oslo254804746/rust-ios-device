# 扩展范围真机补测（2026-10-29）

本轮针对已有信任设备 D2，以 USB 进行补测；受测工作区保留此前未提交改动。
用户授权 DDI、定位模拟与 reset、重启、创建备份、读取崩溃报告、配置查询与导出、
file-relay 和全量 symbols，并在手机端重置信任后重新配对。**备份 Restore 明确排除**，
本轮没有执行 Restore 或变更备份加密。原始设备数据、凭据、路径和内容哈希均留在
仓库外 `20261029-expanded-scope/private/`。

## 当前结果

| 范围 | 已完成的证据 | 边界 |
| --- | --- | --- |
| DDI | Rust CLI attempt136 mount exit 0，137 status 显示已挂载，138 devices 和 139 manifest exit 0，140 auto 幂等 exit 0；144 unmount exit 0，145 status 显示未挂载 | 本设备及当前版本的挂载、查询和卸载证据；不扩展到其他设备或版本 |
| 定位 | 挂载时 Rust CLI attempt141 set、142 GPX、143 reset 均 exit 0 | 当前设备 iOS 17+ Instruments 路径的实际调用；老系统 raw 路径未作本轮真机验收 |
| 完整 symbols | attempt029 使用当时的旧二进制下载 71 个文件、5,152,427,184 字节；与设备目录逐项核对 71 项大小一致 | 原始内容和 SHA256 保存在 private；此结果属于该次二进制，不能自动替代新二进制的全量回归 |
| 备份创建 | 正式 Rust CLI attempt171 自然完成，97.5167 秒、exit 0、stderr 为空，job 已空且输出读取线程已结束；独立核对见下文。参考工具 052 和私有 SDK 单变量对照 168 也自然成功 | 请求带 `--full`，但设备输出的 IsFullBackup=false；仅按实际标志报告。源目录、metadata 哈希清单留在 private，不代表 Restore 已验证 |
| 本地备份读取 | attempt132/133 新 CLI 列出 11,061 条 ID，与独立 SQLite 结果全等，含 1,270 项空域根，重复结果一致且未产生 WAL/SHM；134 单文件提取 159 字节与源精确匹配 | attempt135 全量本地 unback 失败，149 SDK 原始错误为当前平台不支持 symlink 提取；源 metadata 哈希不变，全量展开尚未通过 |
| 设备端备份操作 | attempt155 info、156 list、157 extract-device 均 exit 0。175 修复日期后的 CLI 使用新备份主机副本执行 extract-device，0.84 秒自然 exit 0；独立核对新增唯一输出为 1 个文件、159 字节，与源 payload 逐字节相等 | 176 在同一副本执行 unback-device，30.59 秒自然 exit 1，错误为 UnexpectedEof，原因未确定。177 计划用全新副本作参考工具 Unback 对照，但连接阶段即 DeviceNotFound、USB D2 不可见，Unback 请求未发出，不能算参考工具 Unback 失败。设备端全量 unback 尚缺参考正例。166 的重复提取诊断被强制结束、无数据，不覆盖 157/175 的自然退出 |
| 崩溃报告 | attempt023 pull、024 show、036 pull-all 的精确 basename 结果相互核对 | 025 相对匹配 exit 0 但输出 0 文件，不能算成功匹配；026 watch 约 3 秒 exit 1、无事件，缺正向事件素材 |
| 配置查询 | profiles / provisioning list 为空；027 provisioning dump 为空列表 | 只覆盖空集合分支，show/export 的非空素材尚缺 |
| file-relay | attempt028 及新信任后的 165 均遇服务错误；独立参考工具 034 对同服务得到 ConnectionFailed | 未完成全量传输，不能验收 |
| 重启 | 075 重启请求 exit 0，实际观察 USB 断开、约 47 秒后重连；076 Rust ProductVersion 查询 exit 0 | 仅证明本次设备重新连接并可查询 |
| 信任与重连 | 手机重置信任后 113 Rust 连接失败，参考工具禁用自动配对时观察到未配对；用户在手机确认信任。Windows 保存新记录遇 183，普通文件写入 PermissionDenied，标准 DeletePairRecord 返回 0 但旧文件未更新。主控以仅针对 D2 文件的管理员 helper 同步已验证新记录后，119 Rust info、120 DDI status exit 0；参考工具默认 paired=true，Apple 和 usbmux 记录均匹配新记录 | 验证当时的 USB 参考配对与 Rust 重连，不称 Rust pair 命令通过。172 info exit 0、173 确认 DDI 未挂载、174 显示 WillEncrypt=false；随后 177 连接时 USB 设备不可见，不能据此前结果断言当前连接正常 |

## 备份日期对照与修复

正式 CLI 的旧尝试 016、082 和带 escrow bag 的 147 均 exit 1；147 在用户确认后
自然运行 121.61 秒。私有 SDK trace 159 复现 MBError 205，记录 4,017 次 commit OK、
零 remoteError、零 downloadError，目录请求仅一次。对照 168 只将目录修改日期改成
整秒：自然运行 123.16 秒 exit 0、零 remoteError、4,017 次 commit OK，目录请求增至
257 次，记录的 4,271 个日期均不含小数秒；native backup 与 sync cleanup 均成功。
161 等待手机确认后 600 秒被强制结束且无文件传输，不能作为对照结果；167 已正常
发送 postsyncDidFinish 收尾。

168 的独立核对得到 11,084 条记录、4,011 个普通文件，无缺失且 ID 映射全等；
数据库 quick_check 通过，普通文件共 834,680,523 字节、物理文件 4,015 个，
IsEncrypted=false、IsFullBackup=false。这是单次私有 SDK 增量备份的正向证据。
生产代码已合入该修复，仅对 DeviceLink 修改日期去掉亚秒，并加入实际 XML 回归；localwall、epoch 和
StatusDate 保持原状。[Apple PropertyList DTD](https://github.com/apple-oss-distributions/CF/blob/main/PropertyList.dtd)
描述的日期格式以秒为最小单位；这里的具体互操作判断还依赖本轮真实 XML 字节和
设备端单变量对照，而非仅凭 DTD 推断。

正式 CLI attempt171 使用修复后的程序完成备份。独立验证数据库 quick_check 通过：
11,085 条记录、4,012 个普通文件、缺失 0、ID 映射全等，普通文件共 834,680,573 字节，
物理文件 4,016 个，无 SQLite sidecar；IsEncrypted=false、IsFullBackup=false。
最终源码和二进制与验证记录均匹配。此结果不代表完整 Restore 兼容，Restore 未获授权。

## 实现与主机验证

本轮调整包括 DDI 本地路径、TSS 请求及 ReceiveBytes/MountImage 字段、iOS 17+ 定位
Instruments 路径、backup2 manifest 整数与空域根处理、明文完整文件复制统计、只读
SQLite 副作用和 create 的 escrow bag。真实 DDI 对照及早期失败保留于 attempts
038、049、062、074、081、102–104、125–128；103/104 的 60 秒强制结束发生在
参考脚本持有 mounter 连接期间，后续关闭连接后重新核对。

attempt129 改动范围核心测试 353 通过、1 忽略；130 CLI 测试 52 通过；131 构建成功。
日期修复后 attempt169 backup2 测试 109 通过、1 忽略，170 构建成功；受测程序和完整
哈希见运行目录 `bin/date-fixed-verified.json`，最终源码和程序一致性核对见
`final-source-binary-validation`。主控最终用 `check_protected.py` 确认 `companion/mod.rs`、
`fetchsymbols/mod.rs`、`xpc/h2_raw.rs`、`tests/fetchsymbols.rs` 四个保护文件均与
本轮基线一致；`final-source-review.json` 确认最终源码、程序与已验证构建相符，`git diff --check` 通过。日期修复前源码与 Round 3 一致，此次日期生产修复
仅涉及 `host_fs.rs`。

## 调用覆盖与未决事项

运行前的 138 项未调用规范入口是历史基线，不是通过率。截至 attempt177，对照本轮
CLI 实际业务调用，19 项已调用、119 项仍无调用证据；清单位于运行目录 `reviews/`。
失败、空结果和进行中调用也计作调用；参考工具及 SDK 不计作 CLI 调用。历史分类不
改变本轮授权，也不授权备份 Restore、配置安装等原先禁止的操作。

- 本地全量 unback 的 Windows symlink 限制与设备端 unback-device 的 UnexpectedEof 是
  分开的观察，不能推断前者导致后者；设备端全量 unback 的参考正例尚缺。
- crash watch 需要真实事件，相对匹配需实际产出；配置 show/export 需要非空素材，
  file-relay 需要可用服务。
- 前轮 debugproxy、WebInspector 受控页面、完整壁纸、真实 HCI 和配对手表事件边界
  继续有效；C/Python SDK 全接口、其他 iOS 版本、Wi-Fi 和多设备并发未逐项验收。

最终设备枚举确认旧 USB 目标不可见；用户随后明确确认已移除 iOS 17+ 手机并换上 iOS 14 手机，已另开独立会话测试。未确认拔线的具体时刻，不将先前 EOF 归因于任何设备端操作。正式备份创建及172–174读取验证发生在换机之前。
