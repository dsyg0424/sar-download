# SAR download · 地枢遥感

面向 Sentinel-1 与 NISAR 的 SAR 数据可视化检索与下载桌面应用。在地图上选择研究区，按日期和产品筛选影像，查看覆盖范围并管理下载任务。

采用 **Tauri 2 + Rust + React + TypeScript** 构建。检索和下载由 Rust 原生后端完成，运行时无需安装 Python、Conda、GDAL、Node.js 或 Bun。

[新用户操作指南](docs/新用户操作指南.md) · [支持的产品](#支持的产品) · [快速上手](#快速上手) · [开发与构建](#开发与构建)

## 主要功能

- **地图检索**：卫星底图、影像足迹、轨道与产品树状分组，支持日期、产品、S1 卫星、升降轨和极化筛选。
- **研究区选择**：地图拖动或两点框选、经纬度输入、离线省市县行政区划，以及 GeoJSON、KML、KMZ、Shapefile 边界导入。
- **分批展示结果**：检索过程中显示产品进度和预览；取消或失败时恢复上次完整结果与勾选。
- **下载管理**：单景与批量下载、并发传输、分块下载、暂停与续传、限速、重试和磁盘空间检查。
- **文件校验**：检查长度、本地摘要及适用的文件结构；元数据提供适用的 MD5 / SHA-256 时比对供应商摘要，保存文件时避免覆盖已有成果。
- **任务与账号保存**：重启恢复任务记录，密码存入操作系统凭据管理器；支持清空记录和预览后清理遗留临时文件。
- **结果导出与帮助**：导出所选影像的 GeoJSON，标题栏内置使用帮助。

## 支持的产品

| 数据源 | 产品 | 下载内容 |
| --- | --- | --- |
| Sentinel-1 | SLC | 整景 SLC ZIP 文件 |
| Sentinel-1 | BURST | 单 Burst TIFF 与对应的共享 XML 元数据 |
| NISAR | RSLC | L1 RSLC 整景 HDF5 文件 |
| NISAR | GSLC | L2 GSLC 整景 HDF5 文件 |

检索使用 NASA CMR 公开目录，下载通过相应数据服务完成。产品是否可用取决于目录中的实际数据及账号访问权限。

**研究区用于筛选相交影像，下载文件不会自动裁剪到研究区。** 当前功能集中在整景和单 Burst 下载；多 Burst 合成 SAFE、NISAR 区域裁剪、自动轨道联动及 InSAR / DEM 处理不在当前范围内。

### Sentinel-1 Burst 文件约定

每个新任务下载 `<Burst Granule>.tiff`，以及供 burst2safe 使用的 `<源 SLC Granule>_<极化>.xml`。同一源 SLC、同一极化的多个 Burst 共享 XML，因此 TIFF 与 XML 数量可能不同。

请保持配套 TIFF 和 XML 位于同一目录，并保留原文件名。任务完成前会检查两者的尺寸、索引、子波束、极化、卫星和轨道等对应关系；当前应用不负责将这些文件合成为 SAFE。

## 快速上手

启动桌面应用后：

1. 点击标题栏 **设置**，选择保存目录，填写 Earthdata 用户名和密码，点击 **保存设置**。
2. 通过 **手动框选**、**行政区划**、**边界文件** 或经纬度输入设置研究区。
3. 选择日期和产品类型，按需设置卫星、轨道方向与极化，点击 **检索**。
4. 等待检索结束，核对影像覆盖范围、采集日期和产品信息，再勾选需要的结果。
5. 点击 **下载所选**，在标题栏 **下载** 面板查看进度。
6. 等任务显示 **已完成**，点击任务的文件夹按钮打开保存目录。

公开目录检索无需填写账号；受保护数据下载需要有效的 Earthdata 账号及相应服务授权。账号设置、边界导入和任务操作的完整步骤见 [新用户操作指南](docs/新用户操作指南.md)。

地图上的影像标记与下载复选框分别控制定位和下载选择。收起下载面板不会暂停任务；重启后中断的任务恢复为暂停，需要手动继续。

### 研究区与边界文件

| 方式 / 格式 | 要求 |
| --- | --- |
| 经纬度矩形 | 使用 WGS84 经纬度；西经度小于东经度，南纬度小于北纬度 |
| 行政区划 | 使用行政区外包矩形；需要精确行政边界时导入面文件 |
| GeoJSON / JSON | 包含有效面或多面几何，使用 WGS84 经纬度 |
| KML / KMZ | 包含面边界；KMZ 优先读取 `doc.kml`，否则应只有一个 KML |
| Shapefile | 桌面端选择 `.shp` 后自动读取同目录同名 `.prj`；ZIP 中应只含一套 Shapefile 并包含有效 PRJ |

支持多面和孔洞，不将点线自动转换为面。单次输入文件总量上限为 **20 MiB**，解压后边界上限为 **64 MiB**。跨越 ±180° 经线的研究区请拆分查询。

### 传输设置

| 参数 | 默认值 | 范围 |
| --- | --- | --- |
| 同时下载数 | 3 | 1–32 |
| 单文件连接数 | 8 | 1–64 |
| 限速 | 0（不限速） | 0–10000 MiB/s，整数 |

实际连接数受服务器分段支持、分块数及共享请求预算限制。修改设置后，新启动或继续的任务使用启动时的配置。

## 开发与构建

### 环境准备

项目使用 Bun 管理前端依赖，`package.json` 指定 `bun@1.3.14`；Rust 使用 stable 工具链，Cargo edition 为 2024。

Windows 开发环境还需安装：

- Visual Studio C++ Build Tools 与 Windows SDK。
- Rust MSVC 工具链。
- Microsoft Edge WebView2 运行时。

其他平台需准备对应的 Tauri 系统依赖。下列可执行文件及安装包路径以 Windows 默认构建目录为例。

### 获取源码并启动

```powershell
git clone https://github.com/dsyg0424/sar-download.git
cd sar-download
bun install --frozen-lockfile
bun run tauri dev
```

`tauri dev` 会自动启动 Vite 开发服务器并编译 Rust 后端。Vite 使用固定端口 `1420`，启动前请确认该端口可用。

仅预览前端界面：

```powershell
bun run dev
```

浏览器预览不提供完整的 Rust 检索、下载、系统凭据及原生文件操作能力；验证完整流程请运行桌面应用。

### 检查与测试

```powershell
# TypeScript 检查与前端生产构建
bun run build

# 前端回归测试
bun run test

# Rust 编译检查与回归测试
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

前端测试位于 `tests/`，Rust 测试随模块组织，覆盖筛选与边界解析、结果回滚、任务恢复、续传、完整性校验等行为。

以下测试需联网，默认跳过；CMR 测试只查询公开目录，不下载受保护的影像文件：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml live_catalog_finds_real_fixture_products -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml live_basemaps_and_cache -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml live_vector_resources_and_cache -- --ignored --nocapture
```

### 构建桌面程序

```powershell
# 调试版可执行文件，不生成安装包
bun run tauri build --debug --no-bundle

# 发布构建与安装包
bun run tauri build
```

默认产物位置：

- 调试版程序：`src-tauri/target/debug/sar-download.exe`
- 发布版程序：`src-tauri/target/release/sar-download.exe`
- 安装包目录：`src-tauri/target/release/bundle/`

发布构建将前端静态资源嵌入桌面应用。终端用户无需安装开发工具链；Windows 运行环境需要 WebView2。

## 文件保存与本地数据

启用 **按卫星和产品分类保存** 后，下载目录结构如下：

```text
<保存目录>/
└── SAR/
    ├── Sentinel-1/
    │   ├── SLC/
    │   └── BURST/
    └── NISAR/
        ├── GSLC/
        └── RSLC/
```

关闭分类后直接写入所选目录。更改保存目录只影响新建任务，不会搬移既有文件或任务。

Windows 应用数据保存在 `%LOCALAPPDATA%/com.sar.download/`：

| 位置 | 用途 |
| --- | --- |
| `settings.json` | 保存目录、用户名及传输配置，不含密码 |
| `tasks.json` | 下载任务记录 |
| `downloading/<task-id>/` | 分块状态、续传元数据与适用的校验记录 |
| `map-cache/` | 地图资源磁盘缓存 |

密码由操作系统凭据管理器保存。清空设置中的账号和密码并保存，可移除已保存凭据。

传输中的文件位于各输出目录的 `.sar-download/staging/<task-id>/`。暂停与续传依赖这些文件及任务元数据，请保留仍需继续的任务数据。

**清空记录** 会停止相关任务并移除记录，保留下载成果与续传数据。**空间清理** 会先扫描并展示已移除任务的可清理残留，再按选择删除；仍被任务引用的文件、下载成果、共享 XML 和校验记录会保留。

## 下载与校验说明

- 续传前核对远端文件身份和 Range 支持；身份不可靠或响应长度未知时，可能从头下载。
- 保存前检查文件长度并计算摘要；只有获得适用的供应商摘要时，才能完成对应的 MD5 / SHA-256 比对。
- 根据格式执行轻量结构检查，包括 TIFF / BigTIFF、SLC ZIP / SAFE、HDF5 和 XML；检查结果写入 `verification.json`，Burst 配套关系另写入 `structure.json`。
- **已完成** 表示通过当前任务适用的下载、校验与保存步骤。结构检查不替代完整 ZIP 成员 CRC、HDF5 科学数据验证或影像质量验收。

## 地图与网络

地图使用 ArcGIS World Imagery 卫星影像，**地名与道路** 开关控制 Hybrid Reference 参考层。桌面端地图资源由 Rust 加载并缓存；缓存有效期为 7 天、容量约 256 MiB，仅覆盖浏览过的区域。

底图加载失败时可使用界面中的重试操作，仍可输入经纬度或导入边界开展检索。完整检索和下载需要访问 NASA CMR、Earthdata / ASF 等数据服务，底图需要访问相应 ArcGIS 资源。

## 项目结构

```text
src/
├── App.tsx                      # 应用入口与界面协调
├── features/
│   ├── sar-download/            # 检索表单、结果与状态管理
│   ├── map/                     # 地图、研究区与边界解析 Worker
│   ├── downloads/               # 下载中心、设置与空间清理界面
│   ├── downloader/              # 下载速度等前端模型
│   └── help/                    # 内置使用帮助
└── shared/                     # 通用组件与工具
src-tauri/
├── src/common/                  # 原子写盘、磁盘与请求预算等基础能力
├── src/modules/
│   ├── sar_search/              # CMR 分页、元数据解析与几何筛选
│   ├── downloader/              # 下载、续传、校验与任务服务
│   ├── settings.rs              # 配置、系统凭据与结果导出
│   ├── map_tiles.rs             # 地图资源加载与缓存
│   ├── area.rs                  # 离线行政区划
│   └── boundary.rs              # 原生边界文件选择与读取
├── resources/                   # 离线行政区划数据
└── tauri.conf.json              # 窗口、安全策略与打包配置
tests/                          # 前端测试与公开 CMR 元数据样本
docs/                           # 操作指南与交互检查记录
public/                         # 本地字体与静态资源
scripts/                        # 数据导入等辅助脚本
```

前端负责界面、地图绘制和边界解析；Rust 负责目录检索、下载传输、校验、磁盘操作和凭据管理。检索通过 CMR 分页获取产品，核对目录完整性，并在本地执行多边形相交筛选。

## 常见问题

| 问题 | 处理方式 |
| --- | --- |
| 找不到影像 | 核对产品、日期和研究区，放宽卫星、轨道与极化条件后重试 |
| 取消后本轮结果消失 | 未完成的预览会回滚，恢复上次完整结果和勾选 |
| SHP 导入失败 | 检查同目录同名 PRJ、有效投影和面几何 |
| 下载提示账号或权限错误 | 核对 Earthdata 账号及服务授权，重新保存后继续任务 |
| 字节数已满但任务尚未完成 | 等待校验和最终保存，以任务状态为准 |
| 提示文件已存在 | 核对既有成果与目标目录，应用不会直接覆盖文件 |
| 续传重新从头开始 | 远端身份或分段响应无法可靠验证时会重新下载 |

更多说明见 [新用户操作指南](docs/新用户操作指南.md)。反馈问题时请提供应用版本、产品类型、日期、研究区、任务状态和完整错误文字；请勿附上账号密码。

本地字体 JetBrains Mono 的许可见 [OFL.txt](public/fonts/OFL.txt)，来源信息见 [source.json](public/fonts/source.json)。
