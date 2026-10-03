export const RULES_URL = 'https://rebuildup.dev/share/procon2026';
export const RULE_SECTIONS = {
  map: 'マップ・地形・方向',
  agents: 'エージェント・移動・待機・燃料補給',
  spots: 'スポット・系列・在庫・獲得',
  overview: '競技の目的と勝敗',
  time: '時間・進行・日ごとの引き継ぎ',
  phase: 'ステップ処理・検証・回答の採否',
} as const;
export const rulesLink = (section: string) => `${RULES_URL}#${section}`;
