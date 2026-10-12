// 故意写一份"应该被 ascii-ui-scan 抓住"的英文界面话，用来验那道闸门是不是真的在工作。
// 这不是产品代码，是给闸门当靶子的。抓住 = 闸门有效；放过 = 闸门有盲区。
import { Button, Tooltip, Modal, Empty, Table } from 'antd';

export function DeliberateEnglish(props: { x: unknown }) {
  return (
    <div>
      {/* ① JSX 文本节点 —— 上一版完全不扫这一路 */}
      <Button>Delete</Button>
      <Button>Cancel</Button>
      <span>No results found</span>
      <span>Loading</span>
      <p>Upload failed, please try again</p>

      {/* ② 单个大写词 —— 上一版 words.length < 2 就跳过了 */}
      <Button title="Refresh">R</Button>
      <Empty description="No data" />

      {/* ③ 属性值里的散文 —— 上一版能抓住，这里当回归 */}
      <Tooltip title="Copy the file name to clipboard">
        <span>hint</span>
      </Tooltip>
      <Modal okText="Confirm" cancelText="Go back" open={false} />

      {/* ④ 全是 ALLOW_WORD 拼起来的短语 —— ALLOW 过宽时会漏 */}
      <span>copy path name</span>

      {/* ⑤ 混合大小写标识符，不该误报 */}
      <div className="fs-code path-line" data-testid="pickPane" />
      <span>{String(props.x)}</span>
    </div>
  );
}

export const table = <Table emptyText="Nothing here yet" />;
