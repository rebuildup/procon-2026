import { expect, test } from '@playwright/test';

const properties = [
  'fontFamily',
  'fontSize',
  'fontWeight',
  'lineHeight',
  'color',
  'backgroundColor',
  'borderTopWidth',
  'borderRightWidth',
  'borderBottomWidth',
  'borderLeftWidth',
  'borderTopColor',
  'borderTopLeftRadius',
  'borderTopRightRadius',
  'paddingTop',
  'paddingRight',
  'paddingBottom',
  'paddingLeft',
  'boxShadow',
  'transitionDuration',
  'outlineStyle',
  'outlineWidth',
  'outlineColor',
];

test('labeled replay actions match unmodified Primer buttons in light and dark', async ({
  page,
  context,
}) => {
  for (const mode of ['light', 'dark']) {
    await page.goto('/replays');
    if ((await page.locator('html').getAttribute('data-color-mode')) !== mode)
      await page.getByRole('button', { name: 'Toggle theme' }).click();
    const pane = page.getByRole('complementary', { name: 'Replay information and controls' });
    await expect(pane.getByTestId('replay-toggle')).toHaveText('Play');
    await expect(
      pane.getByRole('group', { name: 'Step navigation' }).getByRole('button'),
    ).toHaveText(['Previous', 'Next']);
    await expect(
      pane.getByRole('group', { name: 'Replay boundaries' }).getByRole('button'),
    ).toHaveText(['First', 'Last']);
    const reference = await context.newPage();
    await reference.goto('/replays');
    if ((await reference.locator('html').getAttribute('data-color-mode')) !== mode)
      await reference.getByRole('button', { name: 'Toggle theme' }).click();
    const markup = await pane.evaluate((el) => el.innerHTML);
    await reference.evaluate((markup) => {
      document
        .querySelectorAll('style[data-vite-dev-id$="/viewer.css"]')
        .forEach((el) => el.remove());
      const fixture = document.createElement('div');
      fixture.innerHTML = markup;
      document.body.replaceChildren(fixture);
    }, markup);
    for (const testId of [
      'replay-toggle',
      'replay-step-back',
      'replay-step-forward',
      'replay-rate-1',
    ]) {
      const actual = page.getByTestId(testId),
        baseline = reference.getByTestId(testId);
      const read = async (locator: typeof actual) =>
        locator.evaluate((el, properties) => {
          const css = getComputedStyle(el);
          return Object.fromEntries(
            properties.map((property) => [property, css[property as keyof CSSStyleDeclaration]]),
          );
        }, properties);
      expect(await read(actual)).toEqual(await read(baseline));
      if (!(await actual.isDisabled())) {
        await page.keyboard.press('Tab');
        await actual.focus();
        await reference.keyboard.press('Tab');
        await baseline.focus();
        expect(await read(actual)).toEqual(await read(baseline));
      }
    }
    const buttons = await pane
      .getByRole('group', { name: 'Step navigation' })
      .getByRole('button')
      .all();
    const previous = (await buttons[0]!.boundingBox())!,
      next = (await buttons[1]!.boundingBox())!;
    expect(Math.abs(previous.y - next.y)).toBeLessThan(1);
    expect(previous.x + previous.width).toBeLessThanOrEqual(next.x + 1);
    await reference.close();
  }
});
