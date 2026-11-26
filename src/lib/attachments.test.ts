import { describe, expect, it } from 'vitest';

import { attachmentKind, attachmentName, isAcceptedFile } from './attachments';

const file = (type: string, name: string) => ({ type, name });

describe('attachmentKind', () => {
  it('classifies picker/clipboard files the same way the accept list does', () => {
    expect(attachmentKind(file('image/png', 'shot.png'))).toBe('image');
    expect(attachmentKind(file('image/jpeg', 'photo.jpg'))).toBe('image');
    expect(attachmentKind(file('text/plain', 'notes.txt'))).toBe('text');
    // 扩展名兜底：剪贴板/拖拽进来的文件 mime 可能为空或不精确。
    expect(attachmentKind(file('', 'main.py'))).toBe('text');
    expect(attachmentKind(file('application/octet-stream', 'config.toml'))).toBe('text');
  });

  it('rejects types outside the accept list', () => {
    expect(attachmentKind(file('application/pdf', 'doc.pdf'))).toBeNull();
    expect(attachmentKind(file('image/svg+xml', 'icon.svg'))).toBeNull();
    expect(attachmentKind(file('', 'archive.zip'))).toBeNull();
    // 视频从未进入模型管线——不接受，避免"可用"的错觉。
    expect(attachmentKind(file('video/mp4', 'clip.mp4'))).toBeNull();
    expect(attachmentKind(file('video/webm', 'rec.webm'))).toBeNull();
    expect(isAcceptedFile(file('application/pdf', 'doc.pdf'))).toBe(false);
    expect(isAcceptedFile(file('image/png', 'shot.png'))).toBe(true);
  });
});

describe('attachmentName', () => {
  const now = new Date(2026, 0, 1, 9, 5, 7);

  it('keeps real file names', () => {
    expect(attachmentName(file('image/png', 'design.png'), now)).toBe('design.png');
  });

  it('names anonymous clipboard blobs so the row is recognizable', () => {
    // 截图粘贴在多数平台给 file.name = "image.png" 或空串。
    expect(attachmentName(file('image/png', 'image.png'), now)).toBe('pasted-090507.png');
    expect(attachmentName(file('image/jpeg', ''), now)).toBe('pasted-090507.jpeg');
    expect(attachmentName(file('', 'blob'), now)).toBe('pasted-090507.bin');
  });
});
