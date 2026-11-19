import { useMemo } from "react";
import type { OntologyEntityType, OntologyRelationType } from "../lib/types";
import { useT } from "../i18n";

/**
 * 実体型 × 実体型のマトリクス。
 *
 * セルは「その (始点, 終点) を結べる関係型」。`*` は任意の型を表すので、
 * ワイルドカードの関係は別枠で数える（そうしないと全セルが埋まって読めない）。
 */
export function OntologyMatrix({
  types,
  relations,
  filter,
  onPick,
}: {
  types: OntologyEntityType[];
  relations: OntologyRelationType[];
  filter: string;
  onPick?: (source: string, target: string) => void;
}) {
  const { t } = useT();

  const shown = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const matched = needle
      ? types.filter(
          (type) =>
            type.name.toLowerCase().includes(needle) ||
            type.display_name_ja.toLowerCase().includes(needle),
        )
      : types;
    // 大きすぎる表は読めないし重い。上限を切って「絞り込め」と伝える。
    return { rows: matched.slice(0, 40), truncated: matched.length > 40 };
  }, [types, filter]);

  const cells = useMemo(() => {
    const index = new Map<string, string[]>();
    const wildcard: string[] = [];
    for (const relation of relations) {
      const sources = relation.source_types.includes("*") ? null : relation.source_types;
      const targets = relation.target_types.includes("*") ? null : relation.target_types;
      if (!sources || !targets) {
        // 片側でも `*` なら全組み合わせに効く。件数だけ別に出す。
        wildcard.push(relation.name);
        continue;
      }
      for (const source of sources) {
        for (const target of targets) {
          const key = `${source}\u0000${target}`;
          const list = index.get(key) ?? [];
          list.push(relation.name);
          index.set(key, list);
        }
      }
    }
    return { index, wildcard };
  }, [relations]);

  if (types.length === 0) {
    return <p className="muted">{t("ontology.matrixEmpty")}</p>;
  }

  return (
    <div className="stack-sm">
      <p className="muted m-0">
        {t("ontology.matrixHint")}
        {shown.truncated ? ` ${t("ontology.matrixTruncated")}` : null}
      </p>
      {cells.wildcard.length > 0 ? (
        <p className="muted m-0">
          {t("ontology.matrixWildcard")}: {cells.wildcard.join(", ")}
        </p>
      ) : null}
      <div className="table-wrap matrix-wrap">
        <table className="data matrix">
          <thead>
            <tr>
              <th className="matrix-corner">→</th>
              {shown.rows.map((type) => (
                <th key={type.name} className="matrix-head" title={type.display_name_ja}>
                  {type.name}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {shown.rows.map((source) => (
              <tr key={source.name}>
                <th className="matrix-row-head" title={source.display_name_ja}>
                  {source.name}
                </th>
                {shown.rows.map((target) => {
                  const names = cells.index.get(`${source.name}\u0000${target.name}`) ?? [];
                  const clickable = Boolean(onPick);
                  return (
                    <td
                      key={target.name}
                      className={`matrix-cell${names.length > 0 ? " matrix-cell-on" : ""}${
                        clickable ? " matrix-cell-clickable" : ""
                      }`}
                      title={
                        names.length > 0
                          ? `${source.name} → ${target.name}: ${names.join(", ")}`
                          : `${source.name} → ${target.name}`
                      }
                      onClick={clickable ? () => onPick?.(source.name, target.name) : undefined}
                    >
                      {names.length > 0 ? names.length : ""}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
