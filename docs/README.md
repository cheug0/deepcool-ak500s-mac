# DeepCool AK500S 数显版 macOS 驱动项目

为黑苹果（Hackintosh）开发九州风神 AK500S DIGITAL（数显版）CPU 散热器的 macOS 兼容驱动，
使其屏幕正常显示 CPU 温度 / 占用率等性能指标。

## 文档目录

| 文档 | 内容 |
|---|---|
| [01-官方软件逆向调研.md](01-官方软件逆向调研.md) | 官方 Windows 软件架构剖析、通信链路、取证过程 |
| [02-通信协议规格.md](02-通信协议规格.md) | **核心文档**：USB HID 报文逐字节定义、设备识别、初始化流程 |
| [03-macOS系统设计.md](03-macOS系统设计.md) | 技术选型、模块设计、传感器数据源（黑苹果适配重点） |
| [04-任务计划.md](04-任务计划.md) | 里程碑、任务分解、测试计划、风险与对策 |
| [05-软件准备清单.md](05-软件准备清单.md) | 开发环境所需的全部软件及用途 |
| [06-阶段0实机验证操作指南.md](06-阶段0实机验证操作指南.md) | hidapitester 实机验证（✅ 已通过，含看门狗发现） |
| [07-阶段1编码架构说明.md](07-阶段1编码架构说明.md) | ak500s-mac 实现架构：模块、设计决策、错误处理矩阵 |
| [08-实机部署测试指南.md](08-实机部署测试指南.md) | 黑苹果部署 → 自检 → 运行 → 验收全流程 |
| [09-通用API接口设计.md](09-通用API接口设计.md) | 任意软件驱动屏幕：NDJSON 协议、TTL 抢占模型、cookbook |

## 工作区目录结构

```
E:\ak500s_mac\
├── docs\          本规划文档
├── analysis\      官方软件逆向分析产物（asar 解包、CAB 提取、扫描脚本）
└── reference\     开源参考项目（已 git clone）
    ├── nortank\                      ★ 协议最全，含 device-list 设备表
    ├── ak500-digital-rs\             ★ Rust + hidapi，代码结构最佳参考
    └── deepcool-ak620-digital-linux\ Python 实现（AK620/AK500S）
```

## 一句话技术路线

用 **Rust + hidapi**（hidapi 原生支持 macOS 的 IOKit HID 后端）复刻已验证的报文协议，
CPU 占用率走 Mach `host_processor_info`，CPU 温度走 **SMC（IOKit SMC User Client）**，
以 launchd 后台守护进程方式常驻，屏幕每 750ms~1s 刷新一次。
