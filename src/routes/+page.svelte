<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { Store } from "@tauri-apps/plugin-store";
  import { save } from "@tauri-apps/plugin-dialog";
  import { onMount, onDestroy } from "svelte";

  type OutputLocation = "source" | "downloads" | "desktop" | "custom";
  type Mode = "archive" | "extract";

  let mode = $state<Mode>("archive");

  let files = $state<string[]>([]);
  let isDragging = $state(false);
  let isProcessing = $state(false);
  let message = $state("");
  let messageType = $state<"success" | "error" | "info" | "">("");

  let outputLocation = $state<OutputLocation>("source");
  let includeParent = $state(true);
  let archiveName = $state("");
  let customOutputPath = $state<string | null>(null);
  let displayOutputPath = $state<string>("");

  // Extract mode state
  let convertTextEncoding = $state(false);

  // Password modal state
  let showPasswordModal = $state(false);
  let passwordInput = $state("");
  let passwordModalError = $state("");
  let passwordResolve: ((value: string | null) => void) | null = null;

  let store: Store | null = null;
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    store = await Store.load("settings.json", {
      defaults: {
        outputLocation: "source",
        includeParent: true,
        convertTextEncoding: false,
      },
      autoSave: true,
    });

    const savedLocation = await store.get<OutputLocation>("outputLocation");
    const savedIncludeParent = await store.get<boolean>("includeParent");
    const savedConvert = await store.get<boolean>("convertTextEncoding");

    if (savedLocation && savedLocation !== "custom") outputLocation = savedLocation;
    if (savedIncludeParent !== null && savedIncludeParent !== undefined) {
      includeParent = savedIncludeParent;
    }
    if (savedConvert !== null && savedConvert !== undefined) {
      convertTextEncoding = savedConvert;
    }

    unlisten = await getCurrentWebview().onDragDropEvent(async (event) => {
      if (event.payload.type === "over") {
        isDragging = true;
      } else if (event.payload.type === "drop") {
        isDragging = false;
        if (mode === "extract") {
          await handleExtractDrop(event.payload.paths);
        } else {
          await handleFileDrop(event.payload.paths);
        }
      } else {
        isDragging = false;
      }
    });
  });

  onDestroy(() => {
    if (unlisten) unlisten();
  });

  function switchMode(next: Mode) {
    if (mode === next) return;
    mode = next;
    // Reset transient state so the two modes don't leak into each other
    clearFiles();
    message = "";
    messageType = "";
  }

  async function saveSettings() {
    if (!store) return;
    if (outputLocation !== "custom") {
      await store.set("outputLocation", outputLocation);
    }
    await store.set("includeParent", includeParent);
    await store.set("convertTextEncoding", convertTextEncoding);
  }

  async function getOutputDir(sourcePath: string): Promise<string> {
    if (outputLocation === "custom" && customOutputPath) {
      return customOutputPath;
    }
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

  async function updateDisplayOutputPath() {
    if (files.length > 0) {
      displayOutputPath = await getOutputDir(files[0]);
    } else {
      displayOutputPath = "";
    }
  }

  // ---------- Archive mode ----------

  async function handleFileDrop(paths: string[]) {
    if (paths.length === 0) return;

    // Single folder: compress immediately
    if (paths.length === 1) {
      const path = paths[0];
      const result = await tryZipFolder(path);
      if (result) return;
    }

    // Files or multiple items: add to list
    const newFiles = paths.filter((p) => !files.includes(p));
    if (newFiles.length > 0) {
      files = [...files, ...newFiles];
      // Set default archive name from first file if not set
      if (!archiveName && files.length > 0) {
        archiveName = getFileName(files[0]);
      }
      await updateDisplayOutputPath();
    }
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
      message = `Created: ${result.output_path}`;
      messageType = "success";
      return true;
    } else if (result.error?.includes("not a folder")) {
      // Return false to add file to list instead
      return false;
    } else {
      message = `Error: ${result.error}`;
      messageType = "error";
      return true;
    }
  }

  async function selectSaveLocation() {
    const result = await save({
      defaultPath: archiveName ? `${archiveName}.zip` : "archive.zip",
      filters: [{ name: "ZIP Archive", extensions: ["zip"] }],
    });

    if (result) {
      // Split full path into parent directory and filename
      const pathParts = result.split("/");
      const fileName = pathParts.pop() || "archive.zip";
      customOutputPath = pathParts.join("/");
      outputLocation = "custom";
      displayOutputPath = customOutputPath;

      // Remove .zip extension and set as archive name
      archiveName = fileName.replace(/\.zip$/i, "");
    }
  }

  async function zipMultipleFiles() {
    if (files.length === 0) return;

    isProcessing = true;
    message = "";

    const outputDir = await getOutputDir(files[0]);
    const finalArchiveName = archiveName || "archive";

    const result = await invoke<{
      success: boolean;
      output_path: string | null;
      error: string | null;
    }>("zip_files", {
      filePaths: files,
      outputDir,
      archiveName: finalArchiveName,
    });

    isProcessing = false;

    if (result.success) {
      message = `Created: ${result.output_path}`;
      messageType = "success";
      clearFiles();
    } else {
      message = `Error: ${result.error}`;
      messageType = "error";
    }
  }

  // ---------- Extract mode ----------

  // Show the password modal and resolve with the entered password, or null if cancelled.
  function promptPassword(errorText = ""): Promise<string | null> {
    passwordInput = "";
    passwordModalError = errorText;
    showPasswordModal = true;
    return new Promise((resolve) => {
      passwordResolve = resolve;
    });
  }

  function submitPassword() {
    showPasswordModal = false;
    const resolve = passwordResolve;
    passwordResolve = null;
    resolve?.(passwordInput);
  }

  function cancelPassword() {
    showPasswordModal = false;
    const resolve = passwordResolve;
    passwordResolve = null;
    resolve?.(null);
  }

  function onPasswordKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      submitPassword();
    } else if (event.key === "Escape") {
      event.preventDefault();
      cancelPassword();
    }
  }

  async function handleExtractDrop(paths: string[]) {
    const zips = paths.filter((p) => p.toLowerCase().endsWith(".zip"));
    if (zips.length === 0) {
      message = "Please drop .zip files";
      messageType = "error";
      return;
    }

    isProcessing = true;
    message = "";
    const extracted: string[] = [];

    try {
      for (const zipPath of zips) {
        const outputDir = await getOutputDir(zipPath);
        let pw: string | null = null;
        let triedPassword = false;

        // Retry loop: prompt for a password only if the archive turns out to be encrypted.
        while (true) {
          const result = await invoke<{
            success: boolean;
            output_path: string | null;
            error: string | null;
            needs_password: boolean;
          }>("unzip_archive", {
            zipPath,
            outputDir,
            password: pw,
            convertTextEncoding,
          });

          if (result.success && result.output_path) {
            extracted.push(result.output_path);
            break;
          }

          if (result.needs_password) {
            const entered = await promptPassword(
              triedPassword ? "Incorrect password. Please try again." : "",
            );
            if (entered === null) {
              break; // User cancelled: skip this archive
            }
            pw = entered;
            triedPassword = true;
            continue;
          }

          // Non-password error: stop and report, keeping any partial successes
          message =
            extracted.length > 0
              ? `Extracted ${extracted.length}, then error: ${result.error}`
              : `Error: ${result.error}`;
          messageType = "error";
          return;
        }
      }
    } finally {
      isProcessing = false;
    }

    if (extracted.length > 0) {
      message =
        extracted.length === 1
          ? `Extracted: ${extracted[0]}`
          : `Extracted ${extracted.length} archives`;
      messageType = "success";
    } else if (!message) {
      // Every dropped archive was cancelled at the password prompt
      message = "Extraction cancelled";
      messageType = "info";
    }
  }

  // ---------- Shared ----------

  function removeFile(index: number) {
    files = files.filter((_, i) => i !== index);
    // Reset archive name when all files are removed
    if (files.length === 0) {
      archiveName = "";
      displayOutputPath = "";
      if (outputLocation === "custom") {
        outputLocation = "source";
        customOutputPath = null;
      }
    }
  }

  function clearFiles() {
    files = [];
    archiveName = "";
    displayOutputPath = "";
    message = "";
    messageType = "";
    if (outputLocation === "custom") {
      outputLocation = "source";
      customOutputPath = null;
    }
  }

  function getFileName(path: string): string {
    return path.split("/").pop() || path;
  }
</script>

<main class="h-screen flex flex-col overflow-hidden">
  <!-- Mode tabs -->
  <div class="flex border-b border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800">
    <button
      onclick={() => switchMode("archive")}
      class="flex-1 py-3 text-sm font-medium transition-colors
        {mode === 'archive'
        ? 'text-blue-600 dark:text-blue-400 border-b-2 border-blue-600 dark:border-blue-400'
        : 'text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200'}"
    >
      Archive
    </button>
    <button
      onclick={() => switchMode("extract")}
      class="flex-1 py-3 text-sm font-medium transition-colors
        {mode === 'extract'
        ? 'text-blue-600 dark:text-blue-400 border-b-2 border-blue-600 dark:border-blue-400'
        : 'text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200'}"
    >
      Extract
    </button>
  </div>

  <!-- Drop zone -->
  <div class="flex-1 flex flex-col items-center justify-center p-4 overflow-hidden">
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
          <p class="text-lg text-gray-600 dark:text-gray-300">Processing...</p>
        </div>
      {:else if mode === "extract"}
        <!-- Extract drop prompt -->
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
              d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"
            />
          </svg>
          <div class="text-center">
            <p class="text-xl font-medium text-gray-600 dark:text-gray-300">
              Drop .zip files to extract
            </p>
            <p class="text-sm text-gray-500 dark:text-gray-400 mt-1">
              Japanese (Shift-JIS) filenames are decoded without mojibake
            </p>
          </div>
        </div>
      {:else if files.length > 0}
        <!-- File list view -->
        <div class="w-full max-w-xl px-4 overflow-y-auto">
          <h2 class="text-lg font-semibold mb-3 text-center">
            Files ({files.length})
          </h2>
          <ul class="space-y-2 max-h-36 overflow-y-auto mb-4">
            {#each files as file, index}
              <li
                class="flex items-center justify-between bg-gray-50 dark:bg-gray-700 rounded-lg px-3 py-2 gap-2"
              >
                <span
                  class="text-xs font-mono text-gray-600 dark:text-gray-300 flex-1 overflow-hidden"
                  style="direction: rtl; text-align: left;"
                  title={file}
                >
                  <bdi>{file}</bdi>
                </span>
                <button
                  onclick={() => removeFile(index)}
                  class="flex-shrink-0 text-red-500 hover:text-red-700 dark:hover:text-red-400"
                  aria-label="Remove"
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

          <!-- Output folder (click to change) -->
          <div class="mb-2">
            <button
              onclick={selectSaveLocation}
              class="text-xs text-blue-600 dark:text-blue-400 hover:underline cursor-pointer overflow-hidden w-full text-left"
              style="direction: rtl; text-align: left;"
              title={displayOutputPath}
            >
              <bdi>{displayOutputPath}/</bdi>
            </button>
          </div>

          <!-- Archive name input -->
          <div class="mb-4">
            <div class="flex gap-2">
              <input
                id="archive-name"
                type="text"
                bind:value={archiveName}
                placeholder="archive"
                class="flex-1 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 text-sm focus:ring-2 focus:ring-blue-500 focus:border-transparent"
              />
              <span class="flex items-center text-gray-500 dark:text-gray-400 text-sm">.zip</span>
            </div>
          </div>

          <div class="flex gap-2 justify-between">
            <button
              onclick={clearFiles}
              class="px-4 py-2 bg-gray-200 hover:bg-gray-300 dark:bg-gray-600 dark:hover:bg-gray-500 rounded-lg font-medium transition-colors"
            >
              Clear
            </button>

            <button
              onclick={zipMultipleFiles}
              class="px-6 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg font-medium transition-colors"
            >
              Create Zip
            </button>
          </div>
        </div>
      {:else}
        <!-- Archive drop prompt message -->
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
              Drop files or folders
            </p>
            <p class="text-sm text-gray-500 dark:text-gray-400 mt-1">
              Folders are zipped instantly, files are added to the list
            </p>
          </div>
        </div>
      {/if}
    </div>

    <!-- Message display -->
    {#if message}
      <div
        class="mt-3 px-4 py-2 rounded-lg text-sm break-all
          {messageType === 'success'
          ? 'bg-green-100 dark:bg-green-900/50 text-green-800 dark:text-green-200'
          : messageType === 'info'
          ? 'bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-200'
          : 'bg-red-100 dark:bg-red-900/50 text-red-800 dark:text-red-200'}"
      >
        {message}
      </div>
    {/if}
  </div>

  <!-- Settings panel -->
  {#if mode === "extract"}
    <div
      class="border-t border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-4"
    >
      <!-- Output location -->
      <div class="mb-3">
        <p class="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
          Output Location
        </p>
        <div class="flex gap-4 flex-wrap">
          <label class="flex items-center gap-2 cursor-pointer">
            <input
              type="radio"
              name="extractOutputLocation"
              value="source"
              bind:group={outputLocation}
              onchange={saveSettings}
              class="text-blue-600"
            />
            <span class="text-sm">Source</span>
          </label>
          <label class="flex items-center gap-2 cursor-pointer">
            <input
              type="radio"
              name="extractOutputLocation"
              value="downloads"
              bind:group={outputLocation}
              onchange={saveSettings}
              class="text-blue-600"
            />
            <span class="text-sm">Downloads</span>
          </label>
          <label class="flex items-center gap-2 cursor-pointer">
            <input
              type="radio"
              name="extractOutputLocation"
              value="desktop"
              bind:group={outputLocation}
              onchange={saveSettings}
              class="text-blue-600"
            />
            <span class="text-sm">Desktop</span>
          </label>
        </div>
      </div>

      <!-- Convert text encoding to UTF-8 -->
      <div class="flex items-center gap-3">
        <label class="relative inline-flex items-center cursor-pointer">
          <input
            type="checkbox"
            bind:checked={convertTextEncoding}
            onchange={saveSettings}
            class="sr-only peer"
          />
          <div
            class="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-blue-300 dark:peer-focus:ring-blue-800 rounded-full peer dark:bg-gray-600 peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-500 peer-checked:bg-blue-600"
          ></div>
          <span class="ml-3 text-sm text-gray-700 dark:text-gray-300"
            >Convert text files to UTF-8 (Shift-JIS → UTF-8)</span
          >
        </label>
      </div>
    </div>
  {:else if files.length === 0}
    <div
      class="border-t border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-4"
    >
    <!-- Output location (shown only when file list is empty) -->
      <div class="mb-3">
        <p class="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
          Output Location
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
            <span class="text-sm">Source</span>
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
            <span class="text-sm">Downloads</span>
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
            <span class="text-sm">Desktop</span>
          </label>
        </div>
      </div>

      <!-- Include parent folder (only effective for folder compression) -->
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
            >Include parent folder when compressing folders</span
          >
        </label>
      </div>
    </div>
  {/if}

  <!-- Password modal (shown only when a dropped archive is encrypted) -->
  {#if showPasswordModal}
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
    >
      <div
        class="w-full max-w-sm bg-white dark:bg-gray-800 rounded-2xl shadow-xl p-5"
      >
        <h2 class="text-lg font-semibold mb-1 text-gray-900 dark:text-gray-100">
          Password required
        </h2>
        <p class="text-sm text-gray-500 dark:text-gray-400 mb-3">
          This archive is encrypted. Enter its password to extract it.
        </p>

        <!-- svelte-ignore a11y_autofocus -->
        <input
          type="password"
          bind:value={passwordInput}
          onkeydown={onPasswordKeydown}
          autofocus
          placeholder="Password"
          class="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 text-sm focus:ring-2 focus:ring-blue-500 focus:border-transparent"
        />

        {#if passwordModalError}
          <p class="text-sm text-red-600 dark:text-red-400 mt-2">
            {passwordModalError}
          </p>
        {/if}

        <div class="flex gap-2 justify-end mt-4">
          <button
            onclick={cancelPassword}
            class="px-4 py-2 bg-gray-200 hover:bg-gray-300 dark:bg-gray-600 dark:hover:bg-gray-500 rounded-lg font-medium transition-colors"
          >
            Cancel
          </button>
          <button
            onclick={submitPassword}
            class="px-6 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg font-medium transition-colors"
          >
            Extract
          </button>
        </div>
      </div>
    </div>
  {/if}
</main>
