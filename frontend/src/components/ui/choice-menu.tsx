'use client';
import type {ComponentProps, ReactNode} from 'react';
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from './select';

type ChoiceProps = {
  value: string | number;
  onValueChange: (value: string) => void;
  'aria-label': string;
  children: ReactNode;
  className?: string;
  disabled?: boolean;
};

// Prefix DOM values so an empty application value (the default screen) is selectable in Radix.
export function ChoiceMenu({value, onValueChange, children, disabled, ...trigger}: ChoiceProps) {
  return <Select value={'=' + value} onValueChange={next => onValueChange(next.slice(1))} disabled={disabled}>
    <SelectTrigger {...trigger}><SelectValue /></SelectTrigger>
    <SelectContent position="popper" collisionPadding={8}>{children}</SelectContent>
  </Select>;
}

export function ChoiceOption({value, ...props}: Omit<ComponentProps<typeof SelectItem>, 'value'> & {value:string|number}) {
  return <SelectItem value={'=' + value} {...props} />;
}
