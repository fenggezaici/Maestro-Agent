import { invoke } from '@tauri-apps/api/core';

/// Native directory picker (live Tauri only). Returns null when cancelled.
export const pickDirectory = () => invoke<string | null>('pick_directory');

/// Native file picker filtered to the given extensions (live Tauri only).
export const pickFile = (extensions: string[]) =>
  invoke<string | null>('pick_file', { extensions });
