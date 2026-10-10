import { theme as antdBuiltin, type ThemeConfig } from 'antd';
import { useSyncExternalStore } from 'react';

/**
 * 设计契约的唯一实现处（`DESIGN.md` 是它的文字版）。一处定义，两处消费：
 * antd 的 `ConfigProvider` 拿这里的 token，CSS 变量由 `styles.css` 再写一份 —— 同名那部分是抄本，
 * 而 `--sel-bg` 与 `--kind-warm` 是**故意不等**色板的两枚（理由写在那两块注释里）——
 * 组件里要的是"取变量/取类名"，不是取字面量（`frontend_theme.rs` 扫这个）。
 */
export const palette = {
  light: {
    canvas: '#FAFAF7', surface: '#FFFFFF', surfaceMuted: '#F6F5F1',
    ink: '#26221B', ink2: '#6B655B', ink3: '#736F67', ink4: '#918D86',
    hairline: '#ECEAE4', hairlineStrong: '#DEDBD2',
    primary: '#0C7E72', onPrimary: '#FFFFFF',
    // 主色的两档交互色也进色板：暗色算法自己派生出来的那一档，白字压上去只有 4.24，
    // 掉在 AA 之下 —— 而"鼠标压在上面的那一瞬"是操作员看得最久的一帧。
    primaryHover: '#0A6C61', primaryActive: '#085B52',
    warning: '#B25E09', danger: '#CD2B33', info: '#2F6FB3',
    // 「会拿走东西」那一颗的两档交互色同样进色板（判据与上面 `primaryHover` 同源：压住时不许比静止更难读）。
    // 不钉的实测：算法派生的 hover 是 `rgb(217,80,82)`，白字压上去 **4.02**（静止那档 5.26）。
    dangerHover: '#B4232B', dangerActive: '#9E1E25',
    // 四档**成片淡底**全部显式给，一档都不交给算法派生。这一族是屏上唯一成片出现的语义底，
    // 而派生值全都卡在"最小那档字（ink3）压上来"的那条线附近：浅色系里 success 派生成
    // rgb(170,189,184)（实测 2.54，整条不过 AA），warning / info 是 4.18 / 4.21（也不过），
    // error 4.55 贴着线。判据：ink3 压在这一格上 ≥4.5 并留余量。数在 `evidence/ui-contrast.md`，
    // 实际渲染值在 `evidence/ui/contrast-r13/`（真窗口，两档 × 三面）。
    successBand: '#F5FAF9', dangerBand: '#FFF8F7', warningBand: '#FBF6EC', infoBand: '#F4F8FA',
  },
  dark: {
    canvas: '#14161A', surface: '#1B1E24', surfaceMuted: '#23272F',
    ink: '#E6E9EF', ink2: '#A2AAB8', ink4: '#60697D',
    // `ink3`（说明/时间那一档）单独记一行：`#7C8598 → #8A93A5`。判据是"这一档字压在**它会出现的所有底**上
    // 都 ≥4.5 且留余量"—— 旧值压卡片底 4.50、压窗口底 4.89，而压**悬停/选中那一行合成后的底**（就是
    // `surfaceMuted` 那一档）只有 **4.04**，而那一层底是表格 `rowHoverBg` 那类半透明叠出来的，
    // 是操作员看得最久的一帧。新值三档变成 5.40 / 5.86 / 4.85，且仍比 `ink2`（6.40–7.74）淡，四档梯子不乱。
    ink3: '#8A93A5',
    hairline: '#262B33', hairlineStrong: '#333A45',
    primary: '#31A99A', onPrimary: '#08130F',
    // 深色这一档主按钮的字是**深墨**（见 Button.primaryColor），所以交互档往亮走反而更安全。
    primaryHover: '#48BCAD', primaryActive: '#5FC9BA',
    warning: '#E0A44A', danger: '#F0808A', info: '#77AEEB',
    // 同一条规则的深色那一半：字是深墨（`dangerColor`），所以交互档往亮走反而更安全。
    dangerHover: '#F59AA2', dangerActive: '#F8B0B6',
    // 深色那一档派生得更狠：error / warning / info 的派生底把 ink3 压到 4.38–4.43，**三档都不过**
    // （浅色系那一次是 2.54 被抓的，深色这一档要同样按实际渲染值量才知道）。
    successBand: '#0D1615', dangerBand: '#221513', warningBand: '#1F1810', infoBand: '#141A22',
  },
} as const;

export type ThemeName = keyof typeof palette;

/**
 * 深色能不能交付，判据是两条，缺一条都不算：色板的配对要**算**过（`tools/ui-contrast.mjs`，
 * 不是截图量的），而 antd 暗色算法自己派生的那些色要在真窗口取**实际渲染值**再算 —— 后者才是
 * 风险面，派生成文字的主色最容易掉到 AA 之下。
 * 这个常量是 kill-switch：`resolveTheme` 在它是 false 时**只会**给浅色，
 * 于是"深色没通过"这件事不依赖有没有人记得把切换口藏起来。
 */
export const DARK_IS_SHIPPED = true;

export function antdTheme(name: ThemeName): ThemeConfig {
  const p = palette[name];
  return {
    algorithm: name === 'dark' ? antdBuiltin.darkAlgorithm : antdBuiltin.defaultAlgorithm,
    token: {
      colorPrimary: p.primary,
      // 交互两档显式给：不写就是算法自己从 primary 派生的那一档，浅色的那一版白字压上去 4.24。
      colorPrimaryHover: p.primaryHover,
      colorPrimaryActive: p.primaryActive,
      colorBgBase: p.surface,
      colorBgContainer: p.surface,
      // 整族浮层的底（模态、抽屉、下拉、气泡卡片）都读这一枚种子：暗色算法把它派生成
      // `rgb(47,57,78)`，自家说明档压上去 3.74（同一句压 `surface` 是 5.40）。
      // 上一片先给 `Modal.contentBg/headerBg/footerBg` 打了三个补丁，这里改成钉种子 ——
      // 一次盖住整族，比哪个组件被量到了就补哪个划算（`DESIGN.md` 的表面三级里写的就是 `surface`＝卡片/弹层）。
      colorBgElevated: p.surface,
      colorBgLayout: p.canvas,
      colorTextBase: p.ink,
      // 四档文字**必须给实色**，不能让算法用透明度近似：它默认派生成 `rgba(ink, .45)` 那一族，
      // 在真窗口按实际渲染值量是浅色 2.76、深色 3.84，两档都掉在 AA 的 4.5 之下 ——
      // 而这一族正是界面上用得最多的那几行（次要说明句、时刻、短注、标签）。
      // 色板里的 ink/ink2/ink3/ink4 就是为这四个层级准备的，值由 `tools/ui-contrast.mjs` 与
      // 真窗口的实际渲染值各自量过一遍。
      colorText: p.ink,
      colorTextSecondary: p.ink2,
      colorTextTertiary: p.ink3,
      colorTextQuaternary: p.ink4,
      // 占位那一档**不给 ink4**：表格里"空着"那句、输入框的提示都是真在传信息的文字，
      // 而算法默认把它落到最淡的一档（浅色 3.30 / 深色 3.03，两档都在 AA 之下）。
      // 给 ink2 而不是 ink3：**旧**那枚深色 ink3（`#7C8598`）压在卡片底上只有 4.50，正好贴着线，
      // 一次 rounding 就翻车。现在的 `#8A93A5` 那档是 5.40，但占位符升一级用 ink2 这件事不改。
      colorTextPlaceholder: p.ink2,
      colorWarning: p.warning,
      colorError: p.danger,
      // 「会拿走东西」那一颗的交互两档：不钉就会露出派生值（浅色 hover 白字 4.02 —— 不量一次不知道）。
      colorErrorHover: p.dangerHover, colorErrorActive: p.dangerActive,
      // 「做完了」就是主色那一档：DESIGN.md 的 colors.success 与 primary 同值，
      // 不设这行会露出 antd 自带的亮绿，和暖白底不是同一套。
      colorSuccess: p.primary,
      // 淡底四档**显式给**（理由见色板那一段）：这一族是屏上唯一成片的语义底，派生值两档都过不了线。
      colorSuccessBg: p.successBand, colorErrorBg: p.dangerBand,
      colorWarningBg: p.warningBand, colorInfoBg: p.infoBand,
      colorInfo: p.info,
      colorLink: p.primary,
      borderRadius: 6,
      borderRadiusSM: 4,
      borderRadiusLG: 12,
      // 字阶与 DESIGN.md 对齐：正文 13.5（body-md）、最小档 11（caption）。
      // 派生档（14-2=12 之类）会造出契约外的第六档，所以 SM 显式给成 caption 那一档。
      fontSize: 13.5,
      fontSizeSM: 11,
      sizeUnit: 4,
      sizeStep: 4,
      wireframe: false,
    },
    components: {
      // `primaryColor` 显式取自色板的 `onPrimary`：暗色算法默认把主按钮的字派生成**白**，
      // 白压在那一档派生出来的中深青上是 3.73，掉在 AA 之下（真窗口量的那一次抓到的就是它）。
      // `dangerColor`（antd 里默认也是白）是同一族的第二个落点：确认层里「我知道，删掉这条」那颗，
      // 白字压派生的浅红 `rgb(207,112,120)` 是 **3.36**、hover 那档 `rgb(229,159,162)` 是 **2.14**
      //（这一格要把确认层也量起来才露得出来 —— 那颗按钮平时不在屏上）。浅色这一档 `onPrimary` 本来就是白，两行对浅色都是空操作。
      Button: { controlHeight: 30, controlHeightSM: 24, contentFontSize: 13, fontWeight: 500, primaryShadow: 'none', primaryColor: p.onPrimary, dangerColor: p.onPrimary },
      Card: { paddingLG: 12 },
      Segmented: { trackBg: p.surfaceMuted, itemSelectedBg: p.surface, borderRadiusSM: 4 },
      Table: { cellPaddingBlockSM: 5, cellPaddingInlineSM: 8, cellFontSizeSM: 12.5, headerBg: p.surface, rowHoverBg: p.surfaceMuted },
      // Sider 自己带底色（antd 的 token，不是类名）：让它跟卡片同值，`.surface` 只管边框/圆角/厚度。
      Layout: { siderBg: p.surface, lightSiderBg: p.surface, bodyBg: p.canvas },
      // 层的底由种子里那条 `colorBgElevated` 管（那里写着派生值与实测数），这里只留圆角。
      Modal: { borderRadiusLG: 16 },
      Alert: { borderRadiusLG: 12 },
    },
  };
}

/**
 * 主题的**选择**与**生效值**是两件事：选「自动」时生效值跟着系统走，选「浅色/深色」时不跟。
 * 把它们混成一个字符串，界面上那颗切换口就会在自己显示"自动"的那一格谎报颜色 ——
 * 而颜色是 `ConfigProvider` 从生效值取的，两处不同步就是"写着自动、其实是硬选的深色"。
 *
 * 状态放在模块级 store，`App` 与顶栏各自订阅，不靠 props 往下传。
 * 理由不是省事：`<AntdApp><Panel /></AntdApp>` 那个形状本身就是一条判据（provider 必须包着消费者），
 * 为了传一个主题名去改那棵树的 props，等于让一个 UI 需求动一条已验证的结构。
 */
export type ThemeMode = 'auto' | 'light' | 'dark';

const STORE_KEY = 'deck.theme';
const MODES: ThemeMode[] = ['auto', 'light', 'dark'];

function storedMode(): ThemeMode {
  try {
    const v = localStorage.getItem(STORE_KEY);
    return MODES.includes(v as ThemeMode) ? (v as ThemeMode) : 'auto';
  } catch {
    return 'auto';
  }
}

/** 系统那一档只有浏览器在才问得出；问不到就按浅色（与模块被非浏览器环境加载时同口径）。 */
const systemDark = (): boolean =>
  typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-color-scheme: dark)').matches;

export function resolveTheme(mode: ThemeMode, dark: boolean): ThemeName {
  if (!DARK_IS_SHIPPED) return 'light';
  return mode === 'auto' ? (dark ? 'dark' : 'light') : mode;
}

let mode: ThemeMode = typeof window === 'undefined' ? 'light' : storedMode();
let current: ThemeName = resolveTheme(mode, systemDark());
const subs = new Set<() => void>();

function apply(): void {
  current = resolveTheme(mode, systemDark());
  document.documentElement.dataset.theme = current;
  subs.forEach((cb) => cb());
}

if (typeof window !== 'undefined') {
  apply();
  // 「自动」要说得出跟的是谁：系统换了就得跟着换，而只在 auto 那一档跟。
  window.matchMedia?.('(prefers-color-scheme: dark)')
    .addEventListener?.('change', () => { if (mode === 'auto') apply(); });
}

/** `[生效值, 选择, 改选择]`：画颜色读生效值，点亮那颗切换口读**选择**。 */
export function useTheme(): [ThemeName, ThemeMode, (next: ThemeMode) => void] {
  const value = useSyncExternalStore(
    (cb) => { subs.add(cb); return () => { subs.delete(cb); }; },
    () => current,
  );
  return [value, mode, (next: ThemeMode) => {
    mode = next;
    try {
      localStorage.setItem(STORE_KEY, next);
    } catch {
      // 存不住就只生效这一次：换主题这件事不该因为存储不可用而被拒。
    }
    apply();
  }];
}
