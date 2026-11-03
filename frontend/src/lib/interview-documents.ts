export type DocumentMetadata = {
  confirmation: 'unreviewed' | 'confirmed' | 'content_changed';
  validity: 'unknown' | 'current' | 'expired' | 'not_yet_valid' | 'content_changed' | 'invalid';
  confirmed_at: string | null; valid_from: string | null; valid_until: string | null; updated_at: string | null;
};
export type DocumentRecord = {path: string; root: string; hash: string; kind: string; allow_external: boolean; chunks: number; metadata: DocumentMetadata};
export type DocumentUpdate = {path: string; expected_root: string; expected_hash: string; confirmed: boolean; valid_from: string | null; valid_until: string | null};
export function sourceStatusLabel(metadata?: DocumentMetadata) {
  if (!metadata) return '核对与时效未标注';
  const confirmation = {unreviewed:'未核对',confirmed:'已核对本次导入',content_changed:'内容已变化，需重新核对'}[metadata.confirmation];
  const validity = {unknown:'时效未设置',current:'在设定有效期内',expired:'已过期',not_yet_valid:'尚未生效',content_changed:'旧日期需重新核对',invalid:'日期无效，需核对'}[metadata.validity];
  return `${confirmation} · ${validity}${metadata.valid_from || metadata.valid_until ? `（${metadata.valid_from || '未设开始'} 至 ${metadata.valid_until || '未设结束'}）` : ''}`;
}
