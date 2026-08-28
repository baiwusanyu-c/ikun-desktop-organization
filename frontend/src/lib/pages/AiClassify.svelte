<script lang="ts">
  import { onMount } from 'svelte';
  import TitleBar from '$lib/components/TitleBar.svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { hideCurrentWindow } from '$lib/window-lifecycle';

  type State = 'confirm' | 'loading' | 'success' | 'error';
  
  let state: State = $state('confirm');
  let errorMessage = $state('');
  let resultMessage = $state('');

  async function cancel() {
    await hideCurrentWindow();
  }

  async function confirm() {
    state = 'loading';
    try {
      await invoke('start_ai_classification');
    } catch (error) {
      state = 'error';
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  onMount(() => {
    // 窗口每次显示时重置状态
    const unsubscribeOpened = listen('window-opened', () => {
      state = 'confirm';
      errorMessage = '';
      resultMessage = '';
    });

    const unsubscribeSuccess = listen('ai-classification-success', () => {
      state = 'success';
      resultMessage = 'AI 分类已完成';
      setTimeout(() => {
        hideCurrentWindow();
      }, 1500);
    });

    const unsubscribeError = listen<string>('ai-classification-error', (event) => {
      state = 'error';
      errorMessage = event.payload;
    });

    return () => {
      unsubscribeOpened.then(fn => fn());
      unsubscribeSuccess.then(fn => fn());
      unsubscribeError.then(fn => fn());
    };
  });
</script>

<div class="window dialog-window">
  <TitleBar title="AI 一键分类" showMinimize={false} onClose={cancel} />
  
  <div class="form-body ai-body">
    {#if state === 'confirm'}
      <p class="message">确定要执行 AI 一键分类吗？</p>
      <p class="hint">该操作会覆盖当前分类布局，不可恢复。</p>
      <footer>
        <button class="secondary" onclick={cancel}>取消</button>
        <button class="primary-button" onclick={confirm}>确定</button>
      </footer>
    {:else if state === 'loading'}
      <div class="loading">
        <div class="spinner"></div>
        <p>正在分析桌面图标...</p>
        <p class="hint">请稍候，AI 正在处理</p>
      </div>
    {:else if state === 'success'}
      <div class="result success">
        <div class="icon">&#10003;</div>
        <p>{resultMessage}</p>
      </div>
    {:else if state === 'error'}
      <div class="result error">
        <div class="icon">&#10007;</div>
        <p>{errorMessage}</p>
        <footer>
          <button class="secondary" onclick={cancel}>关闭</button>
          <button class="primary-button" onclick={confirm}>重试</button>
        </footer>
      </div>
    {/if}
  </div>
</div>

<style>
  .ai-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 20px 24px;
  }

  .message {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-primary);
    margin: 0;
    text-align: center;
  }

  .hint {
    font-size: 12px;
    color: var(--text-secondary);
    margin: 0;
    text-align: center;
  }

  .loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }

  .loading p {
    margin: 0;
    font-size: 13px;
    color: var(--text-primary);
  }

  .spinner {
    width: 32px;
    height: 32px;
    border: 3px solid var(--bg-group);
    border-top-color: var(--primary-color);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .result {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }

  .result p {
    margin: 0;
    font-size: 13px;
    color: var(--text-primary);
  }

  .icon {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 20px;
    font-weight: bold;
    color: white;
  }

  .success .icon {
    background: #10b981;
  }

  .error .icon {
    background: #ef4444;
  }

  footer {
    display: flex;
    gap: 10px;
    margin-top: 8px;
  }
</style>
