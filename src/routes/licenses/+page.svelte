<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import { onMount } from "svelte";

  let notices = $state("");
  let error = $state("");

  onMount(async () => {
    try {
      notices = await invoke<string>("third_party_notices");
    } catch (e) {
      error = String(e);
    }
  });

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      void getCurrentWebviewWindow().close();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<main class="flex h-screen flex-col gap-3 p-4">
  <p class="flex-none text-sm text-neutral-600 dark:text-neutral-400">
    ArcVault bundles the open source libraries listed below, under the licenses shown.
  </p>
  {#if error}
    <p class="text-sm text-red-600 dark:text-red-400">Failed to load the notices: {error}</p>
  {/if}
  <pre
    class="min-h-0 flex-1 overflow-auto whitespace-pre-wrap rounded-md border border-neutral-300 bg-white p-3 font-mono text-[11px] leading-snug select-text dark:border-neutral-700 dark:bg-neutral-950">{notices}</pre>
</main>
