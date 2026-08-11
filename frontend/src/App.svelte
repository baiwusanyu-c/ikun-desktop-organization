<script lang="ts">
  import Router from "svelte-spa-router";
  import ControlPanel from "./lib/pages/ControlPanel.svelte";
  import CreateBox from "./lib/pages/CreateBox.svelte";
  import EditBox from "./lib/pages/EditBox.svelte";

  const routes = {
    "/control-panel": ControlPanel,
    "/create-box": CreateBox,
    "/edit-box": EditBox,
    "*": ControlPanel,
  };
</script>

<main class="app-root">
  <div class="window-shadow">
    <Router {routes} />
  </div>
</main>
