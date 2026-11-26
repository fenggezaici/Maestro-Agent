import type { LocalAttachment } from '../components/LocalAttachments';

export const MAX_ATTACHMENTS = 6;
export const MAX_FILE_SIZE = 20 * 1024 * 1024;
export const IMAGE_INLINE_LIMIT = 3 * 1024 * 1024;
export const TEXT_INLINE_LIMIT = 64 * 1024;

/// 与文件选择器 accept 属性同一份语义；paste/drop 没有 accept 过滤，
/// 必须在代码里兜住不支持的类型。
const IMAGE_MIMES = new Set(['image/png', 'image/jpeg', 'image/webp', 'image/gif']);
const TEXT_EXTENSIONS = new Set([
  '.md', '.txt', '.py', '.ts', '.tsx', '.js', '.jsx', '.json',
  '.rs', '.toml', '.yaml', '.yml', '.log', '.sql', '.sh',
]);

function extension(name: string): string {
  const slash = name.lastIndexOf('/');
  const dot = name.lastIndexOf('.');
  return dot > slash ? name.slice(dot).toLowerCase() : '';
}

export function attachmentKind(file: Pick<File, 'type' | 'name'>): LocalAttachment['type'] | null {
  if (IMAGE_MIMES.has(file.type)) return 'image';
  if (file.type.startsWith('text/') || TEXT_EXTENSIONS.has(extension(file.name))) return 'text';
  return null;
}

export function isAcceptedFile(file: Pick<File, 'type' | 'name'>): boolean {
  return attachmentKind(file) != null;
}

/// 剪贴板文件常常没有可读文件名（截图粘贴为 `image.png` 或空串），
/// 给一个带时间戳的名字让附件行可辨认。
export function attachmentName(file: Pick<File, 'name' | 'type'>, now = new Date()): string {
  if (file.name && file.name !== 'image.png' && file.name !== 'blob') return file.name;
  const ext = file.type.split('/')[1]?.split('+')[0] || 'bin';
  const stamp = `${String(now.getHours()).padStart(2, '0')}${String(now.getMinutes()).padStart(2, '0')}${String(now.getSeconds()).padStart(2, '0')}`;
  return `pasted-${stamp}.${ext}`;
}
