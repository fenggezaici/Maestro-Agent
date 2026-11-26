import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import BrowserPane from './BrowserPane';

afterEach(cleanup);

describe('BrowserPane', () => {
  it('非法地址显示错误且不加载 iframe', () => {
    render(<BrowserPane storageKey="test.browser-url" />);
    const input = screen.getByLabelText('地址栏');
    fireEvent.change(input, { target: { value: 'not a url at all!!' } });
    fireEvent.submit(input.closest('form')!);
    expect(screen.getByRole('alert').textContent).toContain('不是有效的');
    expect(document.querySelector('iframe.browser-frame')).toBeNull();
  });

  it('合法导航渲染 iframe 并持久化', () => {
    render(<BrowserPane storageKey="test.browser-url-2" />);
    const input = screen.getByLabelText('地址栏');
    fireEvent.change(input, { target: { value: 'localhost:3000/dev' } });
    fireEvent.submit(input.closest('form')!);
    const frame = document.querySelector('iframe.browser-frame') as HTMLIFrameElement;
    expect(frame?.src).toBe('http://localhost:3000/dev');
    expect(localStorage.getItem('test.browser-url-2')).toBe('http://localhost:3000/dev');
  });

  it('无 URL 时显示引导占位', () => {
    render(<BrowserPane storageKey="test.browser-url-empty" />);
    expect(screen.getByText('预览本地页面')).toBeTruthy();
  });
});
