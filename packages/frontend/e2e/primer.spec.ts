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
      await page.getByRole('button', { name: '配色を切り替える' }).click();
    const pane = page.getByRole('complementary', { name: '再生情報と操作' });
    await expect(pane.getByTestId('replay-toggle')).toHaveText('再生');
    await expect(
      pane.getByRole('group', { name: 'ステップの移動' }).getByRole('button'),
    ).toHaveText(['前へ', '次へ']);
    await expect(pane.getByRole('group', { name: '再生範囲' }).getByRole('button')).toHaveText([
      '最初へ',
      '最後へ',
    ]);
    const reference = await context.newPage();
    await reference.goto('/replays');
    if ((await reference.locator('html').getAttribute('data-color-mode')) !== mode)
      await reference.getByRole('button', { name: '配色を切り替える' }).click();
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
      const settle = async (locator: typeof actual) =>
        locator.evaluate(async (el) => {
          await new Promise<number>((resolve) => requestAnimationFrame(resolve));
          await Promise.allSettled(el.getAnimations().map((animation) => animation.finished));
        });
      await Promise.all([settle(actual), settle(baseline)]);
      expect(await read(actual)).toEqual(await read(baseline));
      if (!(await actual.isDisabled())) {
        await page.keyboard.press('Tab');
        await actual.focus();
        await reference.keyboard.press('Tab');
        await baseline.focus();
        await Promise.all([settle(actual), settle(baseline)]);
        expect(await read(actual)).toEqual(await read(baseline));
      }
    }
    const buttons = await pane
      .getByRole('group', { name: 'ステップの移動' })
      .getByRole('button')
      .all();
    const previous = (await buttons[0]!.boundingBox())!,
      next = (await buttons[1]!.boundingBox())!;
    expect(Math.abs(previous.y - next.y)).toBeLessThan(1);
    expect(previous.x + previous.width).toBeLessThanOrEqual(next.x + 1);
    await reference.close();
  }
});
