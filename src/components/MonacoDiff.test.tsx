import { render, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import MonacoDiff from './MonacoDiff';

const fake = vi.hoisted(() => ({
  createModel: vi.fn(), createEditor: vi.fn(), setModel: vi.fn(),
  disposeEditor: vi.fn(), disposeModel: vi.fn(), disposeSubscription: vi.fn(),
}));
vi.mock('../editor/monaco', () => ({ monaco: { editor: { createModel: fake.createModel, createDiffEditor: fake.createEditor } } }));

beforeEach(() => {
  fake.createModel.mockImplementation(() => ({ dispose: fake.disposeModel }));
  fake.createEditor.mockImplementation(() => ({
    dispose: fake.disposeEditor, setModel: fake.setModel,
    onDidUpdateDiff: (callback: () => void) => { queueMicrotask(callback); return { dispose: fake.disposeSubscription }; },
  }));
});

it('keeps both models read-only and disposes resources when switching files and unmounting', async () => {
  const view = render(<MonacoDiff split diff={{ path: 'BridgeSession.h', before: 'old', after: 'new' }} />);
  await waitFor(() => expect(fake.createEditor).toHaveBeenCalledTimes(1));
  expect(fake.createModel).toHaveBeenCalledWith('old', 'cpp');
  expect(fake.createModel).toHaveBeenCalledWith('new', 'cpp');
  expect(fake.createEditor).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ readOnly: true, originalEditable: false, renderSideBySide: true, diffAlgorithm: 'advanced' }));
  view.rerender(<MonacoDiff split={false} diff={{ path: 'next.ts', before: 'before', after: 'after' }} />);
  await waitFor(() => expect(fake.createEditor).toHaveBeenCalledTimes(2));
  expect(fake.disposeEditor).toHaveBeenCalledTimes(1);
  expect(fake.disposeModel).toHaveBeenCalledTimes(2);
  expect(fake.createEditor.mock.calls[1][1]).toMatchObject({ renderSideBySide: false });
  view.unmount();
  expect(fake.disposeEditor).toHaveBeenCalledTimes(2);
  expect(fake.disposeModel).toHaveBeenCalledTimes(4);
  expect(fake.disposeSubscription).toHaveBeenCalledTimes(2);
});

it('provides a readable fallback if the editor cannot be initialized', async () => {
  fake.createEditor.mockImplementationOnce(() => { throw new Error('editor unavailable'); });
  const view = render(<MonacoDiff split diff={{ path: 'example.h', before: 'old text', after: 'new text' }} />);
  await waitFor(() => expect(view.getByRole('status')).toHaveTextContent('只读文本差异'));
  expect(view.getByLabelText('提议代码')).toHaveTextContent('new text');
  view.unmount();
  expect(fake.disposeModel).toHaveBeenCalledTimes(2);
});
