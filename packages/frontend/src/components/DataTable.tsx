import { For, type JSX } from 'solid-js';
import styles from '@/vendor/primer/Table.module.css';

export function DataTable(props: {
  label: string;
  headers: readonly string[];
  columns: string;
  scroll?: boolean;
  children: JSX.Element;
}) {
  return (
    <div
      class={`${styles.TableOverflowWrapper} data-table`}
      data-scroll={props.scroll ? true : undefined}
      tabindex={props.scroll ? 0 : undefined}
      role={props.scroll ? 'region' : undefined}
      aria-label={props.scroll ? `${props.label}（横スクロール）` : undefined}
    >
      <table
        role="table"
        class={styles.Table}
        data-cell-padding="condensed"
        style={{ '--grid-template-columns': props.columns }}
        aria-label={props.label}
      >
        <thead role="rowgroup" class={styles.TableHead}>
          <tr role="row" class={styles.TableRow}>
            <For each={props.headers}>
              {(header) => (
                <th role="columnheader" scope="col" class={styles.TableHeader}>
                  {header}
                </th>
              )}
            </For>
          </tr>
        </thead>
        <tbody role="rowgroup" class={styles.TableBody}>
          {props.children}
        </tbody>
      </table>
    </div>
  );
}
export function DataRow(props: { children: JSX.Element }) {
  return (
    <tr role="row" class={styles.TableRow}>
      {props.children}
    </tr>
  );
}
export function DataCell(props: {
  children: JSX.Element;
  heading?: boolean;
  title?: string;
  end?: boolean;
}) {
  return (
    <>
      {props.heading ? (
        <th
          role="rowheader"
          scope="row"
          class={styles.TableCell}
          data-cell-align={props.end ? 'end' : 'start'}
        >
          <span class="data-cell-content" title={props.title}>
            {props.children}
          </span>
        </th>
      ) : (
        <td role="cell" class={styles.TableCell} data-cell-align={props.end ? 'end' : 'start'}>
          <span class="data-cell-content" title={props.title}>
            {props.children}
          </span>
        </td>
      )}
    </>
  );
}
