import type { LocalAttachment } from '../components/LocalAttachments';

export const MAX_ATTACHMENTS = 6;
export const MAX_FILE_SIZE = 20 * 1024 * 1024;
export const IMAGE_INLINE_LIMIT = 3 * 1024 * 1024;
export const TEXT_INLINE_LIMIT = 64 * 1024;
/// Anthropic 视觉输入的最优长边——更大的图不增加识别质量却成倍消耗 token。
export const IMAGE_MAX_EDGE = 1568;
/// 小图按字节直通原编码：避免无意义重编码损耗，动图 GIF 也得以保留；
/// 1 MiB 以内的图即使尺寸略超最优长边，token 代价也可接受。
export const IMAGE_PASSTHROUGH_BYTES = 1024 * 1024;

/// 等比缩放到长边不超过 maxEdge 的目标尺寸；已在预算内则原样返回。
export function scaledDimensions(width: number, height: number, maxEdge = IMAGE_MAX_EDGE): [number, number] {
  const longest = Math.max(width, height);
  if (longest <= maxEdge) return [width, height];
  const scale = maxEdge / longest;
  return [Math.max(1, Math.round(width * scale)), Math.max(1, Math.round(height * scale))];
}

function readAsDataUrl(blob: Blob): Promise<string | undefined> {
  return new Promise((resolve) => {
    const reader = new FileReader();
    reader.onload = () => resolve(typeof reader.result === 'string' ? reader.result : undefined);
    reader.onerror = () => resolve(undefined);
    reader.readAsDataURL(blob);
  });
}

/// 图片发送前处理：超过 IMAGE_PASSTHROUGH_BYTES 的图解码重采样到
/// IMAGE_MAX_EDGE 长边并按 webp 0.85 重编码（webp 保留 alpha，WebView2/Chromium
/// 均支持编码）；小图按字节直通原编码——避免无意义重编码，动图 GIF 也得以保留。
/// 压缩后仍超 IMAGE_INLINE_LIMIT 或解码失败返回 undefined——调用方按"未发送"处理。
export async function prepareImageDataUrl(file: File): Promise<string | undefined> {
  if (file.size <= IMAGE_PASSTHROUGH_BYTES) {
    return await readAsDataUrl(file);
  }
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    return undefined;
  }
  try {
    const [width, height] = scaledDimensions(bitmap.width, bitmap.height);
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext('2d');
    if (!context) return undefined;
    context.drawImage(bitmap, 0, 0, width, height);
    const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/webp', 0.85));
    if (!blob || blob.size > IMAGE_INLINE_LIMIT) return undefined;
    return await readAsDataUrl(blob);
  } finally {
    bitmap.close();
  }
}

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
