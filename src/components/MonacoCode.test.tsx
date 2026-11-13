import { render, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import MonacoCode from './MonacoCode';

const fake = vi.hoisted(() => ({ create: vi.fn(), createModel: vi.fn(), update: vi.fn(), disposeEditor: vi.fn(), disposeModel: vi.fn() }));
vi.mock('../editor/monaco', () => ({ monaco: { editor: { create: fake.create, createModel: fake.createModel } } }));

it('creates a folding-enabled readonly file viewer, updates appearance and releases resources', async () => {
  fake.createModel.mockImplementation(() => ({ dispose: fake.disposeModel }));
  fake.create.mockImplementation(() => ({ dispose: fake.disposeEditor, updateOptions: fake.update, setModel: vi.fn() }));
  const file = { id: 'mock-file', path: 'src/File.cpp', content: 'void test() {}', source: 'mock' as const };
  const view = render(<MonacoCode file={file} fontSize={12} fontFamily="Consolas" />);
  await waitFor(() => expect(fake.create).toHaveBeenCalledTimes(1));
  expect(fake.create).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ readOnly: true, folding: true, automaticLayout: false }));
  view.rerender(<MonacoCode file={file} fontSize={16} fontFamily="Cascadia Code" />);
  await waitFor(() => expect(fake.update).toHaveBeenCalledWith(expect.objectContaining({ fontSize: 16 })));
  expect(fake.create).toHaveBeenCalledTimes(1);
  view.unmount();
  expect(fake.disposeEditor).toHaveBeenCalledTimes(1);
  expect(fake.disposeModel).toHaveBeenCalledTimes(2);
});
