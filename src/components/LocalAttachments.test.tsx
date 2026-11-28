import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import LocalAttachments, { type LocalAttachment } from './LocalAttachments';

// 1×1 透明 gif（67 字节 base64）
const GIF = 'data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==';

const image: LocalAttachment = { id: 'a', name: 'pasted.png', type: 'image', url: GIF, dataUrl: GIF };

describe('LocalAttachments injection receipt', () => {
  it('shows injection receipt with volume for sent images', () => {
    render(<LocalAttachments items={[image]} sent />);
    const receipt = screen.getByText(/已注入模型上下文/);
    expect(receipt.textContent).toContain('KiB');
  });

  it('reads natural dimensions into the receipt after image load', () => {
    render(<LocalAttachments items={[image]} sent />);
    const img = document.querySelector('.attachment-preview img') as HTMLImageElement;
    Object.defineProperty(img, 'naturalWidth', { value: 1920 });
    Object.defineProperty(img, 'naturalHeight', { value: 1080 });
    fireEvent.load(img);
    expect(screen.getByText(/已注入模型上下文/).textContent).toContain('1920×1080');
  });

  it('keeps honest unsent labelling when image has no dataUrl', () => {
    render(<LocalAttachments items={[{ ...image, dataUrl: undefined }]} sent />);
    expect(screen.getByText(/图片未发送/)).toBeTruthy();
    expect(screen.queryByText(/已注入模型上下文/)).toBeNull();
  });

  it('keeps plain sent label for text attachments', () => {
    render(<LocalAttachments items={[{ id: 't', name: 'notes.md', type: 'text', url: '' }]} sent />);
    expect(screen.getByText('已随消息发送')).toBeTruthy();
  });
});
