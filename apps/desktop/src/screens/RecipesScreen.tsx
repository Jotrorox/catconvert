import { type Component, Show } from "solid-js";
import { RecipeGrid } from "../components/RecipeGrid";
import { RECIPES } from "../lib/formats";
import { state } from "../lib/store";

export const RecipesScreen: Component = () => (
  <div class="page">
    <header class="page-header">
      <div>
        <h1 class="display page-title">Recipes</h1>
        <p class="subtitle">Pick a recipe, then the files. Everything stays tweakable.</p>
      </div>
    </header>
    <RecipeGrid large recipes={RECIPES} />
    <section>
      <h2 class="section-title">Your recipes</h2>
      <Show
        when={state.recipes.length > 0}
        fallback={
          <p class="item-meta empty-note">
            Dial in a file in the editor, then choose “Save these settings as a recipe”.
          </p>
        }
      >
        <RecipeGrid large recipes={state.recipes} />
      </Show>
    </section>
  </div>
);
