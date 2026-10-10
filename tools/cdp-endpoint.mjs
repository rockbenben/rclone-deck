// 真窗口 CDP 的端口从哪儿来。三个工具（验收驱动、颜色尺子、单发驱动器）共用这一份口径。
//
// 为什么不写死 9222：本机实测过 `netsh interface ipv4 show excludedportrange protocol=tcp`
// 把 **9211-9310** 整段留走了（一次重启之后落进动态排除区）。旧写法请求 9222 时 Chromium
// **一个字都不说、一个端口都不听** —— 窗口照常开，驱动那边只得到"面板起来了吗"，
// 读起来像界面坏了，而坏的是尺子借用的那一段端口。
//
// 所以启动侧（`tools/ui-launch.cmd`）给 `--remote-debugging-port=0` 让系统挑，
// 这里读 Chromium 自己写在 WebView2 用户目录里的 `DevToolsActivePort` 第一行。
// 每次调用都重读一遍：面板重启后那个号会变，缓存的那一份就是下一次"连不上"的假象。
import fs from 'node:fs';
import path from 'node:path';

/** WebView2 那份用户目录的名字来自 `tauri.conf.json` 的 identifier（不是另抄一份应用名）。 */
function portFile() {
  const conf = JSON.parse(fs.readFileSync(path.join(process.cwd(), 'src-tauri', 'tauri.conf.json'), 'utf8'));
  const id = conf.identifier ?? conf?.app?.identifier;
  if (!id) throw new Error('tauri.conf.json 里没有 identifier，认不出 WebView2 用户目录');
  const local = process.env.LOCALAPPDATA ?? process.env.AppData;
  return path.join(local, id, 'EBWebView', 'DevToolsActivePort');
}

export function cdpPort() {
  const given = process.argv.find((a) => a.startsWith('--port='))?.split('=')[1]
    ?? process.env.DECK_CDP_PORT;
  if (given) return given;
  const file = portFile();
  let text = '';
  try {
    text = fs.readFileSync(file, 'utf8');
  } catch {
    throw new Error(`读不到 ${file}：面板起来了（并带 --remote-debugging-port=0）才会有这一份。`
      + ' 已知端口可以用 --port= 或环境变量 DECK_CDP_PORT 直接给。');
  }
  const port = text.split('\n')[0].trim();
  if (!/^\d+$/.test(port)) throw new Error(`${file} 第一行不是端口号：${JSON.stringify(port)}`);
  return port;
}

export const cdpEndpoint = () => `http://127.0.0.1:${cdpPort()}/json/list`;
