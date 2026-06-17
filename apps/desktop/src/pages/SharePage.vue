<script setup lang="ts">
import UnderConstruction from "../components/UnderConstruction.vue";
import { CAPABILITIES } from "../lib/nav";

const m = CAPABILITIES.find((c) => c.path === "/share")!;
</script>

<template>
  <UnderConstruction :name="m.name" :icon="m.icon" :desc="m.desc" :version="m.version" />
</template>
