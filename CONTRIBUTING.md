# 参与 rclone-deck 的开发

面向第一次在这份代码上动手的人。要读设计契约去 [DESIGN.md](DESIGN.md)，这里只讲**怎么把代码改对、把闸门跑绿**。
（代码与设计契约里那些 `docs/superpowers/**`、`evidence/**`、`spec §N`、裁定 `R##` 的坐标是**本机的历史读数**，不随仓发布 —— `.gitignore` 里写着为什么。它们是"这条判据量过才定"的出处，不是链接；要重取读数就重跑那把尺子。）

## 先决条件

| 东西 | 版本 | 为什么是它 |
| --- | --- | --- |
| Node | **≥ 24**（CI 跑 24） | `package.json` 的 `engines` 钉的下限，也是 CI 验的那一支 |
| Rust | stable，带 clippy | 由 [rust-toolchain.toml](rust-toolchain.toml) 钉住，`cargo` 会自己装 |
| rclone | 本机装好的 v1.75.1 | 测试起的是**真** `rclone rcd`，不是 mock |
| WebView2 | Windows 10/11 自带 | Tauri 2 的 WebView |

## 装与跑

```powershell
npm install                 # 前端依赖（package-lock.json 是准的）
npm run tauri -- dev        # 起开发面板
npm run tauri build         # 出安装包：src-tauri/target/release/bundle/nsis/
```

## 闸门

本地要绿的，和 [.github/workflows/ci.yml](.github/workflows/ci.yml) 里跑的是同一批命令 —— CI 不是另一套规矩，只是把同一批命令换到干净的机器上再跑一遍。

| 命令 | 管什么 | CI 对应 |
| --- | --- | --- |
| `npm run typecheck` | `tsc --noEmit` | frontend |
| `npm run build` | typecheck + `vite build` | frontend（打包也走它） |
| `npm run copy:scan` | 文案标点（直引号括中文 / 中文句中的半角冒号） | frontend |
| `npm audit --audit-level=high` | npm 侧已知漏洞 | frontend |
| `npm run lint:rust` | `cargo clippy --all-targets --features test-helpers` | rust |
| `npm test` | 全套 Rust 测试（含真后台） | rust |
| `cargo audit --file src-tauri/Cargo.lock` | Rust advisory 数据库 | security |

一次性跑完：`npm run build`、`npm run copy:scan`、`npm run lint:rust`、`npm test`。

### 文案扫描：它是闸门，但它只扫界面

`npm run copy:scan` 现在**标点有违规就退 1**（清单条数不算违规 —— 那是活儿没干完，不是规矩破了），并已接进 `ci.yml` 的 `frontend` job。在这之前它永远退 0，`DESIGN.md` 那句「标点机械保证零违规」只能靠人读输出。

**射程要说清楚**：它只读 `src/**` 与 `src-tauri/src/**` 的 `.ts/.tsx/.rs`——界面文案。**README、`docs/`、配置文件都不在里面**。2026-10-09 通盘时量到 `DESIGN.md` 自己就有 134 处 ASCII 直引号（叙述性行文，与界面文案是两回事）。**界面零违规 ≠ 全仓零违规。**

改界面上的一句话想看差在哪：清单每轮重生成，与上一份对一遍（生成命令在脚本头部；落点在 `docs/superpowers/` 下，**不入库**）。改一条就在差份里记一条。）

### 验收驱动器是**本机**工具，不是 CI 闸门

```powershell
node tools/ui-acceptance.mjs --home=A --from=0 --to=20 --out=docs/superpowers/evidence/ui/<名字>
```

它在**真窗口**里逐动作走一遍，每屏一张 PNG 加一行 DOM 读数，判定必须读 `<out>/readings.txt` 而不是 stdout（含中文的行会被终端回显吞掉，`Read` 那个文件是唯一可信的读法）。

接不进 CI 的原因是硬的：它要一个起着的面板（CDP）、一个夹具家（`%TEMP%\deck-ui-A` / `-B`），而脚本里那两处是**本机绝对路径**，远端名按夹具家写死成操作员配置里的 `fxa` / `fxb`。先按 `tools/ui-launch.cmd` 的规矩起面板（`RCLONE_DECK_HOME` 必须写在启动命令自己的环境里，见 `evidence/acceptance.md` 记的那两次误启动）。

**跑它之前有三个坑（2026-10-09 每一个都踩了）：**

1. **必须在仓根跑。** `tools/cdp-endpoint.mjs` 从 `process.cwd()` 找 `src-tauri/tauri.conf.json`，`--out` 也是相对路径 —— 在别处跑要么找不到模块，要么把报告写到别处去。
2. **debug 版面板要 `npm run dev` 开着。** `tauri.conf.json` 里 debug 走 `devUrl: http://localhost:1420`、`release` 走 `frontendDist`。用 `cargo build` 出来的 debug exe **没起 vite 就会开出一张 `ERR_CONNECTION_REFUSED` 的白窗**，而驱动器 30 秒后报的是「界面还没挂上（没有"配好的位置"那个分段控件）」—— 读起来像界面坏了，坏的是**没开的前端**。要么先 `npm run dev`，要么用 `tools\ui-launch.cmd A src-tauri\target\release\rclone-deck.exe`（那个把 `dist/` 嵌进去了）。
3. **改了前端要重新 `npm run build`。** Tauri 把 `dist/` **编进 exe**，前端改了不重编，跑的还是旧界面。

**rcd 的端口不用你管**（2026-10-10 起）。两处各挑一次，用的是同一套判据：

| 位置 | 干什么 |
|---|---|
| `config/mod.rs` → `AppConfig::pick_port` | **面板自己**的默认端口（新用户第一次启动就走这条） |
| `tools\free-rcd-port.ps1` | 验收夹具/启动器那条通路 |
| `tests/localdaemon/mod.rs`、`tests/rcd_supervisor.rs` | 套件里的夹具号 |

以前那个写死的 6400 在 2026-10-10 已经被排除区（6383–6482）盖住了。

**判据是「真去绑一下」，不是查表。** 查表会漏两样，而两样的症状**长得一模一样**：

| 漏掉的 | 这台机的值 | 症状 |
|---|---|---|
| 动态（临时）端口范围 `netsh int ipv4 show dynamicport tcp` | **1024 起 / 13977 个**（不是默认的 49152 起） | 挑中的号随时被系统抓去 |
| 别人占着，且状态是 `Bound` 不是 `Listen` | 6483 被 pid 5512 占 | 只查 Listen 会当成空的 |

两种都报同一句 `bind: ... access permissions`（WSAEACCES），面板顶栏写「还没连上后台」，
**读起来像界面坏了，坏的是一个绑不上的号**。所以脚本先避开动态段与排除区（只用来收窄窗口），
再用**绑得上**这一条事实来定。排除区会继续往上漂，所以抬高默认值根治不了。

要自己钉一个端口仍然可以，设 `RCLONE_DECK_PORT` 就会原样用你的值，不改。

### 这颗雷已经踩过三次了

同一个病（端口在 Windows 的**动态排除区**里 → `bind` 报 WSAEACCES → 一整组用例红，而病因与被测代码无关）：

| 什么时候 | 谁 | 排除区 | 怎么死的 |
|---|---|---|---|
| 2026-10-09 | 生产默认 5599 + 夹具 5598 | 5460–5659 | 两个号一起被吃，三个 target 整组红 |
| 2026-10-09 | 抬到 6400 + 6399 | 5460–5659 | 抬完就好了（**只管了一天**） |
| 2026-10-10 | 还是 6400 + 6399 | 漂到 **6383–6482** | 抬的数字又被吃了，同一批 target 再红一次 |

**所以"抬高默认值"是挪雷，不是修。** 现在两处都改成现挑：
`tools\free-rcd-port.ps1`（面板启动器）与 `tests/localdaemon/mod.rs` 的 `PORT` /
`tests/rcd_supervisor.rs` 的 `default_port()`（Rust 侧直接 `TcpListener::bind` 试一下）。

**判据统一成「真去绑一下」**：被占 bind 失败，落在排除区里 bind **同样**失败 ——
两种病因在判据上是同一件事，所以不必解析 `netsh` 那两张表（`excludedportrange` 与
`dynamicport` 是两套机制，2026-10-10 那次只查第一套就漏了第二套）。

⚠️ **生产默认 6400 也不再是写死的那个号**（`config/mod.rs` 的 `AppConfig::pick_port`）——
但它**认得两种"绑不上"**，这是关键：

| 状态 | 判据 | 处置 |
|---|---|---|
| `Free` | 绑得上 | 用 6400 |
| `Taken` | 绑不上，但**有人在听** | **不换号**，交回 6400，让启动那一步按 `reclaim` 自证并说出占用者是谁 |
| `Unusable` | 绑不上，**也没人听** | 换号 —— 这是动态排除区，Windows 不许任何进程绑它 |

第二行是 `rcd/mod.rs` 明写的「**也不静默换端口**」：那句话说里有操作员要的东西，换号会把它吞掉。
第三行才是今天修的那个洞。

另外 `pick_port` 的**扫描起点固定、逐个往后**（15000 起），所以同一台机器上多个面板算出**同一个号**
—— 排除区是机器的属性而不是"谁在跑"的属性。这样 `reclaim` 靠 pid 记录的单实例闸才继续成立；
若改成"随便找个空的"，两个面板会各拿一个号、谁也看不见谁，于是起两个 rcd。

顺带：`vite` 默认只绑 `::1`，量它的时候探 `localhost` 或 `[::1]`，探 `127.0.0.1` 会得到一个假的「连不上」。

### 三条会让闸门给出假绿的规矩

这三条都不是风格偏好，是**跑过之后红过**才写成这样的：

1. **`npm test` 必须带 `--test-threads=1`。** 脚本里已经带了，别绕开脚本直接敲 `cargo test`。[Cargo.toml](src-tauri/Cargo.toml) 里那五条 `[[test]]` 目标（`rcd_supervisor` / `runner_local` / `mirror_runner` / `browser_runner` / `draft_pairs`）**每一条都要绑一个真后台端口**：四条共用 `tests/localdaemon` 那份夹具（一个进程里现挑一次），`rcd_supervisor` 自己 `default_port()` 现挑。并发跑的表现是 `address already in use` 或端口档空转，那不是「端口脏了」，是调度串了。

2. **clippy 必须带 `--features test-helpers`。** 那五条 `[[test]]` 都带 `required-features`；不开这个 flag，cargo 会**静默跳过**它们 —— 看着全绿，其实最该看的那五个文件根本没进编译。`test-helpers` 只给测试开出口（如 `rcd::command_line_of`），生产构建里它不存在。

3. **验证前端源码的证人住在 Rust 侧。** [src-tauri/tests/](src-tauri/tests/) 里有几条直接读 `src/*.ts` 的源码来判形状（经 [tests/common/mod.rs](src-tauri/tests/common/mod.rs) 的 `strip_comments`）。所以**改了前端也得跑 `npm test`**，只跑 `npm run build` 漏掉它们。它们按 `CARGO_MANIFEST_DIR` 定位文件，所以在哪个目录敲 cargo 都能找到。

## 为什么不跑 `cargo fmt`

**因为这份 Rust 代码是手排的，`cargo fmt` 会把它拆散。** 大量地方把相关实参或链式调用压在一条上，例如 [src-tauri/src/commands.rs](src-tauri/src/commands.rs) 里 `JobView::new(` 后面那四行实参 —— 默认排版会展开成十几行，读者要在十几行里找「这一趟带了哪几个数」。

实测数字（2026-10-09）：默认配置下 `cargo fmt --check` 报 **1379 处**；即使在 [rust-toolchain.toml](rust-toolchain.toml) 旁边加一份 `use_small_heuristics = "Max"`，仍有 **1033 处**。差的这 346 处是链宽，剩下的是「一个结构体字面量里塞两三个字段」这类手排偏好，rustfmt 没有开关能让它接受。

所以：**CI 不把 fmt 当闸门**，`rust-toolchain.toml` 里也不装 rustfmt。想统一排版的那一天，应该是单独一个「全量重排 + 逐条人读」的提交，而不是混在别的改动里。

## 依赖怎么进

**现在没有依赖自动更新**：仓里没有 `.github/dependabot.yml`（只有 `ci.yml`），升级靠人跑。
要开自动 PR 再补那份配置；届时的分组口径先记在这里，免得开的时候现编：devDependencies 一组、运行期依赖一组、Rust 生态一组 —— 前两组各自内部互相咬合（`vite` / `@vitejs/plugin-react` / `typescript` 是同一波发版），拆开只会让人手工排版本组合。

两条锁文件都**入库**，别删：`package-lock.json` 与 [src-tauri/Cargo.lock](src-tauri/Cargo.lock)。CI 走 `npm ci`，锁文件与 `package.json` 不一致会直接红。

> **`typescript` 已经迁到 `^7`**（7.0.2，Go 原生重写那一支）。这次是一次独立迁移，不是混在依赖更新里升的：`tsc --noEmit` 与 `vite build` 都验过，Rust 侧那七条读前端源码的证人（`frontend_*`）也重跑过（当时全绿；条数别抄在这里，跑一次 `npm test` 现量）。`@types/react@19` 与 TS 7 实测相容，没动。
>
> 升之前**先量**、升之后**照样量**：TS 7 是大版本，`tsc` 的检查项与诊断措辞都可能变，真红了要能说清是「代码本来就有问题」还是「新版本查得更严」。

## 行尾

[.gitattributes](.gitattributes) 把文本统一成 **LF 入库、LF 检出**，二进制显式钉死，`.cmd` / `.bat` / `.ps1` 单独用 CRLF（cmd.exe 按 LF 写出的 `.cmd` 在部分主机上行尾解析会出问题）。

**历史不用动。** 量过 `HEAD` 里的 blob：全部本来就是 LF —— 索引里从来没有过 CRLF。工作区里那几个 CRLF 文件（2026-10-10 现量：119 个跟踪文件里 6 个）是本机 `core.autocrlf=true` 检出时转出来的**显示效果**，不是仓库里的内容。实测 `git add --renormalize` 对未被修改的干净文件**stage 了 0 个**，印证了这一点。

所以 `eol=lf` 这条真正做到的事是：它**盖掉** `core.autocrlf`，往后新检出 / 新提交的工作区一律是 LF，那批 CRLF 自然收敛，且 `git status` 不会因此多出任何改动 —— 不用专门做一次「行尾规范化」提交。

> 换机器或换克隆路径后如果又看到满屏 `CRLF will be replaced by LF`，那是 `.gitattributes` 在起作用（提示下次 git 碰它时转换），**不是**改动丢了。

## CI 跑在哪

`.github/workflows/ci.yml`，五条 job：

| job | 跑在哪 | 什么时候 | 装什么 |
| --- | --- | --- | --- |
| `frontend` | windows-latest | 每个 push / PR | Node 24 + `npm ci` |
| `rust` | windows-latest | 每个 push / PR | Rust + **rclone**（五条测试目标要起真后台） |
| `security` | ubuntu-latest | 每个 push / PR | `cargo-audit` |
| `bundle` | windows-latest | 推 main / 手动 | 出 NSIS 安装包并传成 artifact |
| `release` | windows-latest | 打 `v*` 标签 | 出安装包**与更新包**，并建一个 release |

两点说明：

- **只有 Windows runner。** 本仓只支持 Windows（README「支持范围」写死），加 linux/macos 绿了也证明不了任何事。
- **`bundle` 不在 PR 上跑。** NSIS 要现下、release 模式要重编全量 Rust；PR 上「能编过」由 `rust` 那条 job 兜住。手动触发（workflow_dispatch）可以单独出安装包。

Rust 编译缓存走 [Swatinem/rust-cache](https://github.com/Swatinem/rust-cache)，工作区显式指成 `src-tauri -> .` —— `Cargo.toml` 不在仓根，不指的话每轮都要从头编 tauri。

### 发一个版本：安装包与更新包

面板「设置」那面有一项**「版本与更新」**：默认**关着**，开了才会在每次打开时联网问一句有没有新版；
也可以随时点「立即检查」。有新版时面板只说版本号——下载与安装要人自己按。

它读的是 GitHub 上的 `releases/latest/download/latest.json`，所以发一个版本要走三步：

1. **先把版本号改对**：`src-tauri/tauri.conf.json` 与 `package.json` 里的版本号要一样，
   并且要等于打算打的那个标签（`v0.1.1` 那种）。`release` 那条 job 用的 `tagName: v__VERSION__`
   拿的是配置里那一个版本，对不上 tauri-action 会当场拒——错位的标签会让更新包指向一个装不上的版本。
2. **打标签**：`git tag v0.1.1` 然后推上去。这一趟 `release` 那条 job 出安装包与更新包
   （`latest.json` 加各个 `.sig`），并建一个 release。
3. **把那个 release 发布出去**（草稿 → 发布）。更新端点读的是 `releases/latest`，
   而它**只认已发布的** release：草稿里的 `latest.json` 外面看不到。
   这一步是最后一步，也是唯一需要人按的那一步。

**私钥在哪儿**：仓里存的是**公钥**（`src-tauri/rclone-deck.pub`，公钥本来就是公开的）。
私钥只在本机（`src-tauri/.updater/`，那个目录不入库），并且要存成仓的 secret
**`TAURI_SIGNING_PRIVATE_KEY`**（值是那整个 PEM 文本）。这把钥匙**没有密码**，
所以不需要 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（留一个空值即可）。

`bundle.createUpdaterArtifacts` 开着的时候，**出包必须有私钥**——没有就出不了更新包，
`bundle` 与 `release` 两条 job 都会红在这上面。所以这个 secret 要先建好再推 main。

## 踩过的坑

- **`target/` 里存着改名之前的东西。** 这份目录从 `365/041` 搬到 `365/041-rclone-deck` 时，`src-tauri/target` 一起搬了过来，于是 build 脚本产物里**硬编码着旧路径**。cargo 的指纹不把 target 目录的绝对路径算进去，所以它不会自己失效，结果是 `cargo check` / `npm run tauri build` 直接红在 `failed to read plugin permissions` 上，看起来像配置坏了，其实是缓存坏了。
  **两个 profile 各中一次**（2026-10-09 实测）：`target/debug/build` 下 17 个、`target/release/build` 下 7 个。只修一个 profile 的话，另一个会在下一次跑那条命令时才现形 —— 所以**先全树扫一遍再动手**，别等第二个 profile 自己冒出来。查法（`output` 文件里存着 build 脚本的输出，旧路径就写在那儿）：

  ```powershell
  Get-ChildItem src-tauri\target -Recurse -Filter output -File |
    Where-Object { (Get-Content $_.FullName -Raw) -match '365\\041\\src-tauri' } |
    ForEach-Object { Split-Path $_.FullName -Parent }
  ```

  最省事还是搬完目录就 `cargo clean`（本仓 target 有 29 GB，重编十几分钟）。
- **npm 镜像不实现 audit 端点。** 用 npmmirror 这类镜像跑 `npm audit` 会得到 `/-/npm/v1/security/* not implemented` —— 那是**假绿**，不是「没有漏洞」。CI 走官方源。
