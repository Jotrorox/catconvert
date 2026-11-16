import { type Component, For, Show } from "solid-js";
import type { Recipe } from "../lib/formats";
import { allRecipes, chooseFiles, removeRecipe } from "../lib/store";
import { Icon } from "./Icon";

/** Recipe cards: pick one, then pick files, and the editor opens preset. */
export const RecipeGrid: Component<{ large?: boolean; recipes?: readonly Recipe[] }> = (props) => (
  <div class="recipes" classList={{ large: props.large }}>
    <For each={props.recipes ?? allRecipes()}>
      {(recipe, index) => (
        <div class="recipe-slot">
          <button
            type="button"
            class="recipe"
            data-tone={recipe.tone}
            style={{ "--i": index() }}
            onClick={() => void chooseFiles(recipe.id)}
          >
            <span class="recipe-title">{recipe.title}</span>
            <span class="recipe-detail">{recipe.detail}</span>
            <span class="recipe-route">{recipe.route}</span>
          </button>
          <Show when={recipe.custom}>
            <button
              type="button"
              class="recipe-remove"
              aria-label={`Remove ${recipe.title}`}
              title="Remove recipe"
              onClick={() => removeRecipe(recipe.id)}
            >
              <Icon name="close" size={14} />
            </button>
          </Show>
        </div>
      )}
    </For>
  </div>
);
