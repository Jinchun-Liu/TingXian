export type StructuredSelection = { root: string; file_hash: string; pointers: string[] };
export type StructuredField = { pointer: string; kind: string; preview: string; characters: number; children: number };
export type StructuredPreview = { root: string; path: string; file_hash: string; fields: StructuredField[] };
export const isStructuredPath = (path: string) => /\.(json|ya?ml)$/i.test(path);
export const includesField = (parent: string, child: string) => parent === '' || child === parent || child.startsWith(parent + '/');
