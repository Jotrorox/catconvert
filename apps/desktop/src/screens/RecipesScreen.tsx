import type { Component } from "solid-js";
import { RecipeGrid } from "../components/RecipeGrid";

export const RecipesScreen: Component = () => (
  <div class="page">
    <header class="page-header">
      <div>
        <h1 class="display page-title">Recipes</h1>
        <p class="subtitle">Pick a recipe, then the files. Everything stays tweakable.</p>
      </div>
    </header>
    <RecipeGrid large />
  </div>
);
