<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { LicensePackage } from "@/components/about/data";
import {
  fetchLicenses,
  filterPackages,
  groupLicenses,
} from "@/components/about/data";
import PackageRow from "@/components/about/PackageRow.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { ChevronDown, ChevronRight, Search, TriangleAlert } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const loading = ref(true);
const data = ref<Awaited<ReturnType<typeof fetchLicenses>> | null>(null);
const query = ref("");

// The filter is a local pass over a few hundred strings (sub-millisecond), so
// it runs directly per keystroke — no debounce. (EntryListPage debounces
// because its search hits the backend; this one doesn't.)

// Expansion is driven by reactive Sets rather than native <details> so that a
// closed group's packages — and an unopened package's license text — never
// enter the DOM. With hundreds of entries that each carry a multi-KB license,
// this is what keeps the tab light: only the open group's rows and only the
// expanded package's text are ever rendered.
const expandedGroups = ref(new Set<string>());
const expandedPkgs = ref(new Set<string>());

onMounted(async () => {
  // fetchLicenses is failure-tolerant: it resolves to a degraded doc (empty
  // packages, complete:false) rather than throwing, so we distinguish "fetch
  // failed" from "loaded but empty" via `data.complete` below.
  data.value = await fetchLicenses();
  loading.value = false;
});

const packages = computed<LicensePackage[]>(() => data.value?.packages ?? []);
const total = computed(() => packages.value.length);
const rustCount = computed(() => data.value?.ecosystems.rust ?? 0);
const npmCount = computed(() => data.value?.ecosystems.npm ?? 0);
const degraded = computed(() => !!data.value && !data.value.complete);
// Fetch failure surfaces as a degraded doc with 0 packages + a note; a genuine
// empty inventory (complete, 0 packages) is separate.
const failed = computed(
  () => total.value === 0 && !!data.value && !data.value.complete,
);

const trimmedQuery = computed(() => query.value.trim());
const searching = computed(() => trimmedQuery.value.length > 0);

// Grouped view (no active search) — collapsed-by-default license groups. Cached
// purely on `packages`; the template decides whether to render it.
const groups = computed(() => groupLicenses(packages.value));

// Flat results view (active search).
const results = computed<LicensePackage[]>(() =>
  searching.value ? filterPackages(packages.value, trimmedQuery.value) : [],
);

/** Stable, collision-free key for a package (names may contain "@" / "/"). */
function pkgKey(p: LicensePackage): string {
  return `${p.ecosystem}|${p.name}|${p.version}`;
}

function toggle<T>(set: Set<T>, value: T) {
  if (set.has(value)) set.delete(value);
  else set.add(value);
  // Reassign to trigger Vue reactivity on the Set.
  return new Set(set);
}
function toggleGroup(license: string) {
  expandedGroups.value = toggle(expandedGroups.value, license);
}
function togglePkg(p: LicensePackage) {
  expandedPkgs.value = toggle(expandedPkgs.value, pkgKey(p));
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <!-- Search + summary -->
    <div class="search-bar">
      <BaseIcon :icon="Search" :size="16" class="search-icon" />
      <BaseInput
        v-model="query"
        type="search"
        class="search-input"
        :placeholder="t('about.licenses.searchPlaceholder')"
        :aria-label="t('about.licenses.searchAria')"
      />
    </div>

    <div v-if="loading" class="state-row">
      <BaseSpinner /><span class="text-sm text-muted">{{
        t("about.licenses.loading")
      }}</span>
    </div>

    <BaseAlert v-else-if="failed" variant="danger">
      {{ t("about.licenses.loadFailed") }}
    </BaseAlert>

    <BaseAlert v-else-if="total === 0" variant="danger">
      {{ t("about.licenses.empty") }}
    </BaseAlert>

    <template v-else>
      <p class="summary text-sm">
        <strong>{{ t("about.licenses.summary", { count: total }) }}</strong>
        <span class="text-muted">
          {{
            t("about.licenses.summaryEco", {
              rust: rustCount,
              npm: npmCount,
            })
          }}
        </span>
      </p>

      <BaseAlert
        v-if="degraded"
        variant="warning"
        class="flex items-start gap-2"
      >
        <BaseIcon :icon="TriangleAlert" :size="16" class="shrink-0 mt-0.5" />
        <span>{{ t("about.licenses.degradedNotice") }}</span>
      </BaseAlert>

      <!-- Grouped view -->
      <div v-if="!searching" class="flex flex-col gap-2">
        <section v-for="group in groups" :key="group.license" class="group">
          <button
            type="button"
            class="group-head"
            :aria-expanded="expandedGroups.has(group.license)"
            @click="toggleGroup(group.license)"
          >
            <BaseIcon
              :icon="
                expandedGroups.has(group.license) ? ChevronDown : ChevronRight
              "
              :size="16"
            />
            <span class="group-license">{{ group.license }}</span>
            <span class="group-count">{{
              t("about.licenses.packageCount", group.count)
            }}</span>
          </button>
          <ul v-if="expandedGroups.has(group.license)" class="pkg-list">
            <PackageRow
              v-for="p in group.packages"
              :key="pkgKey(p)"
              :pkg="p"
              :expanded="expandedPkgs.has(pkgKey(p))"
              @toggle="togglePkg(p)"
            />
          </ul>
        </section>
      </div>

      <!-- Search results (flat) -->
      <div v-else class="flex flex-col gap-2">
        <p v-if="results.length === 0" class="text-sm text-muted">
          {{ t("about.licenses.noResults", { query: trimmedQuery }) }}
        </p>
        <ul v-else class="pkg-list">
          <PackageRow
            v-for="p in results"
            :key="pkgKey(p)"
            :pkg="p"
            :expanded="expandedPkgs.has(pkgKey(p))"
            @toggle="togglePkg(p)"
          />
        </ul>
      </div>
    </template>
  </div>
</template>

<style scoped>
.search-bar {
  position: relative;
  display: flex;
  align-items: center;
}
.search-icon {
  position: absolute;
  left: 0.6rem;
  color: var(--color-muted, var(--color-edge));
  pointer-events: none;
}
/* Pad the input so text clears the leading search icon. */
.search-bar :deep(input) {
  padding-left: 2rem;
}
.state-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 1rem 0;
}
.summary {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  align-items: baseline;
  padding: 0 0.25rem;
}
.group {
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  overflow: hidden;
}
.group-head {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.6rem 0.7rem;
  background: var(--color-surface);
  border: 0;
  cursor: pointer;
  text-align: left;
  -webkit-tap-highlight-color: transparent;
}
.group-head:active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .group-head:hover {
    background: var(--color-hover);
  }
}
.group-license {
  flex: 1;
  font-size: var(--text-sm);
  font-weight: 600;
  word-break: break-all;
}
.group-count {
  font-size: var(--text-xs);
  color: var(--color-muted, var(--color-edge));
  white-space: nowrap;
}
.pkg-list {
  list-style: none;
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--color-edge);
}
.pkg-list > li {
  border-bottom: 1px solid var(--color-edge);
}
.pkg-list > li:last-child {
  border-bottom: 0;
}
</style>
