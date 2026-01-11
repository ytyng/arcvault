<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { Store } from "@tauri-apps/plugin-store";
  import { onMount, onDestroy } from "svelte";

  type OutputLocation = "source" | "downloads" | "desktop";

  let files = $state<string[]>([]);
  let isDragging = $state(false);
  let isProcessing = $state(false);
  let message = $state("");
  let messageType = $state<"success" | "error" | "">("");

  let outputLocation = $state<OutputLocation>("source");
  let includeParent = $state(true);

  let store: Store | null = null;
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    // Store を初期化
    store = await Store.load("settings.json", {
      defaults: {
        outputLocation: "source",
        includeParent: true,
      },
      autoSave: true,
    });

    const savedLocation = await store.get<OutputLocation>("outputLocation");
    const savedIncludeParent = await store.get<boolean>("includeParent");

    if (savedLocation) outputLocation = savedLocation;
    if (savedIncludeParent !== null && savedIncludeParent !== undefined) {
      includeParent = savedIncludeParent;
    }

    // Tauri のドラッグ&ドロップイベントをリッスン
    unlisten = await getCurrentWebview().onDragDropEvent(async (event) => {
      if (event.payload.type === "over") {
        isDragging = true;
      } else if (event.payload.type === "drop") {
        isDragging = false;
        await handleFileDrop(event.payload.paths);
      } else {
        isDragging = false;
      }
    });
  });

  onDestroy(() => {
    if (unlisten) unlisten();
  });

  async function saveSettings() {
    if (!store) return;
    await store.set("outputLocation", outputLocation);
    await store.set("includeParent", includeParent);
  }

  async function getOutputDir(sourcePath: string): Promise<string> {
    switch (outputLocation) {
      case "downloads":
        return (await invoke<string | null>("get_downloads_dir")) || sourcePath;
      case "desktop":
        return (await invoke<string | null>("get_desktop_dir")) || sourcePath;
      default:
        return (
          (await invoke<string | null>("get_parent_dir", { path: sourcePath })) ||
          sourcePath
        );
    }
  }

  async function handleFileDrop(paths: string[]) {
    if (paths.length === 0) return;

    // 単一アイテムの場合
    if (paths.length === 1) {
      const path = paths[0];
      // パスの末尾が / でないかつ拡張子がなければフォルダの可能性が高い
      // Rust 側でフォルダかどうかを判定するので、とりあえず zip_folder を試す
      const result = await tryZipFolder(path);
      if (result) return;
    }

    // 複数アイテムまたは単一ファイルの場合はリストに追加
    files = [...files, ...paths.filter((p) => !files.includes(p))];
  }

  async function tryZipFolder(folderPath: string): Promise<boolean> {
    isProcessing = true;
    message = "";

    const outputDir = await getOutputDir(folderPath);

    const result = await invoke<{
      success: boolean;
      output_path: string | null;
      error: string | null;
    }>("zip_folder", {
      folderPath,
      outputDir,
      includeParent,
    });

    isProcessing = false;

    if (result.success) {
      message = `作成完了: ${result.output_path}`;
      messageType = "success";
      return true;
    } else if (result.error?.includes("フォルダが存在しません")) {
      // フォルダではなかったので false を返す
      return false;
    } else {
      message = `エラー: ${result.error}`;
      messageType = "error";
      return true;
    }
  }

  async function zipMultipleFiles() {
    if (files.length === 0) return;

    isProcessing = true;
    message = "";

    const outputDir = await getOutputDir(files[0]);
    const archiveName = `archive_${new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19)}`;

    const result = await invoke<{
      success: boolean;
      output_path: string | null;
      error: string | null;
    }>("zip_files", {
      filePaths: files,
      outputDir,
      archiveName,
    });

    isProcessing = false;

    if (result.success) {
      message = `作成完了: ${result.output_path}`;
      messageType = "success";
      files = [];
    } else {
      message = `エラー: ${result.error}`;
      messageType = "error";
    }
  }

  function removeFile(index: number) {
    files = files.filter((_, i) => i !== index);
  }

  function clearFiles() {
    files = [];
    message = "";
    messageType = "";
  }

  function getFileName(path: string): string {
    return path.split("/").pop() || path;
  }
</script>

<main
  class="h-screen flex flex-col"
>
  <!-- ドロップゾーン -->
  <div class="flex-1 flex flex-col items-center justify-center p-4">
    <div
      class="w-full h-full border-4 border-dashed rounded-2xl flex flex-col items-center justify-center transition-all duration-200
        {isDragging
        ? 'border-blue-500 bg-blue-50 dark:bg-blue-900/30'
        : 'border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800'}"
    >
      {#if isProcessing}
        <div class="flex flex-col items-center gap-4">
          <div
            class="w-12 h-12 border-4 border-blue-500 border-t-transparent rounded-full animate-spin"
          ></div>
          <p class="text-lg text-gray-600 dark:text-gray-300">処理中...</p>
        </div>
      {:else if files.length > 0}
        <!-- ファイルリスト表示 -->
        <div class="w-full max-w-md px-4">
          <h2 class="text-lg font-semibold mb-3 text-center">
            ファイル一覧 ({files.length}件)
          </h2>
          <ul class="space-y-2 max-h-48 overflow-y-auto mb-4">
            {#each files as file, index}
              <li
                class="flex items-center justify-between bg-gray-50 dark:bg-gray-700 rounded-lg px-3 py-2"
              >
                <span class="truncate text-sm" title={file}
                  >{getFileName(file)}</span
                >
                <button
                  onclick={() => removeFile(index)}
                  class="ml-2 text-red-500 hover:text-red-700 dark:hover:text-red-400"
                  aria-label="削除"
                >
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    class="h-5 w-5"
                    viewBox="0 0 20 20"
                    fill="currentColor"
                  >
                    <path
                      fill-rule="evenodd"
                      d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z"
                      clip-rule="evenodd"
                    />
                  </svg>
                </button>
              </li>
            {/each}
          </ul>
          <div class="flex gap-2 justify-center">
            <button
              onclick={zipMultipleFiles}
              class="px-6 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg font-medium transition-colors"
            >
              Zipを作成
            </button>
            <button
              onclick={clearFiles}
              class="px-4 py-2 bg-gray-200 hover:bg-gray-300 dark:bg-gray-600 dark:hover:bg-gray-500 rounded-lg font-medium transition-colors"
            >
              クリア
            </button>
          </div>
        </div>
      {:else}
        <!-- ドロップ促進メッセージ -->
        <div class="flex flex-col items-center gap-4">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            class="h-16 w-16 text-gray-400 dark:text-gray-500"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="1.5"
              d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12"
            />
          </svg>
          <div class="text-center">
            <p class="text-xl font-medium text-gray-600 dark:text-gray-300">
              ファイルまたはフォルダをドロップ
            </p>
            <p class="text-sm text-gray-500 dark:text-gray-400 mt-1">
              フォルダは即座にZip化、ファイルはリストに追加されます
            </p>
          </div>
        </div>
      {/if}
    </div>

    <!-- メッセージ表示 -->
    {#if message}
      <div
        class="mt-3 px-4 py-2 rounded-lg text-sm
          {messageType === 'success'
          ? 'bg-green-100 dark:bg-green-900/50 text-green-800 dark:text-green-200'
          : 'bg-red-100 dark:bg-red-900/50 text-red-800 dark:text-red-200'}"
      >
        {message}
      </div>
    {/if}
  </div>

  <!-- 設定パネル -->
  <div
    class="border-t border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-4"
  >
    <!-- 出力先 -->
    <div class="mb-3">
      <p class="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
        保存先
      </p>
      <div class="flex gap-4 flex-wrap">
        <label class="flex items-center gap-2 cursor-pointer">
          <input
            type="radio"
            name="outputLocation"
            value="source"
            bind:group={outputLocation}
            onchange={saveSettings}
            class="text-blue-600"
          />
          <span class="text-sm">元の場所</span>
        </label>
        <label class="flex items-center gap-2 cursor-pointer">
          <input
            type="radio"
            name="outputLocation"
            value="downloads"
            bind:group={outputLocation}
            onchange={saveSettings}
            class="text-blue-600"
          />
          <span class="text-sm">ダウンロード</span>
        </label>
        <label class="flex items-center gap-2 cursor-pointer">
          <input
            type="radio"
            name="outputLocation"
            value="desktop"
            bind:group={outputLocation}
            onchange={saveSettings}
            class="text-blue-600"
          />
          <span class="text-sm">デスクトップ</span>
        </label>
      </div>
    </div>

    <!-- 親フォルダを含める -->
    <div class="flex items-center gap-3">
      <label class="relative inline-flex items-center cursor-pointer">
        <input
          type="checkbox"
          bind:checked={includeParent}
          onchange={saveSettings}
          class="sr-only peer"
        />
        <div
          class="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-blue-300 dark:peer-focus:ring-blue-800 rounded-full peer dark:bg-gray-600 peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-500 peer-checked:bg-blue-600"
        ></div>
        <span class="ml-3 text-sm text-gray-700 dark:text-gray-300"
          >フォルダ圧縮時、元のフォルダを含めて圧縮する</span
        >
      </label>
    </div>
  </div>
</main>
