import { App as AntdApp, ConfigProvider } from 'antd';
import zhCN from 'antd/locale/zh_CN';

import { Panel } from './Panel';
import { antdTheme, useTheme } from './theme';

/**
 * 只做两件事：把主题令牌交给 antd，再把 `<AntdApp>` 摆在**消费者外面**（裁定 R57 第一条）。
 *
 * 之前 `AntdApp.useApp()` 就写在渲染 `<AntdApp>` 的同一个组件里，provider 在消费者**自己**
 * 的子树里，antd 只能给回兜底上下文——`message.error` 不是函数，面板从此一条拒绝都没说过。
 * antd 文档的形状就是：provider 在外，用 `useApp()` 的那一层在内。
 * 不换成静态 `message.error(...)`（不走主题与上下文），也不自绘 toast。
 *
 * `ConfigProvider` 也在 `<AntdApp>` 外面：它给的是令牌，不是上下文，但同样不能被消费者自己包。
 */
export default function App() {
  const [theme] = useTheme();
  return (
    <ConfigProvider locale={zhCN} theme={antdTheme(theme)}>
      <AntdApp>
        <Panel />
      </AntdApp>
    </ConfigProvider>
  );
}
