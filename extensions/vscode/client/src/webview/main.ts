import { mount } from 'svelte';
import type { ViewKind } from '../shared/protocol';
import App from './App.svelte';
import './theme.css';

mount(App, {
    target: document.body,
    props: { view: document.body.dataset.view as ViewKind },
});
