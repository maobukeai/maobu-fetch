import React, { useState, useRef, useEffect, useLayoutEffect, useId, ReactNode } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, Check } from "lucide-react";
import { nextEnabledIndex } from "./select-navigation";

export interface SelectOption<T extends string | number = string | number> {
  value: T;
  label: string | ReactNode;
  disabled?: boolean;
}

export interface SelectProps<T extends string | number = string | number> {
  value: T;
  onChange: (value: T) => void;
  options: SelectOption<T>[];
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  style?: React.CSSProperties;
  ariaLabel?: string;
}

export function Select<T extends string | number = string | number>({
  value,
  onChange,
  options,
  placeholder = "请选择...",
  disabled = false,
  className = "",
  style,
  ariaLabel,
}: SelectProps<T>) {
  const [isOpen, setIsOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const listId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const [coords, setCoords] = useState<{ top: number; left: number; width: number; placeAbove: boolean }>({
    top: 0,
    left: 0,
    width: 0,
    placeAbove: false,
  });

  const selectedOption = options.find((opt) => opt.value === value);
  const open = () => {
    const selected = options.findIndex((option) => option.value === value && !option.disabled);
    setActiveIndex(selected >= 0 ? selected : nextEnabledIndex(options, -1, 1));
    setIsOpen(true);
  };

  useEffect(() => {
    if (isOpen && activeIndex >= 0) document.getElementById(`${listId}-${activeIndex}`)?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, isOpen, listId]);

  // 计算定位（更新 top/left/width 以及是否向上弹出）
  const updateCoords = () => {
    if (!triggerRef.current) return;
    const rect = triggerRef.current.getBoundingClientRect();
    const dropdownHeight = Math.min(options.length * 32 + 12, 240);
    const spaceBelow = window.innerHeight - rect.bottom;
    const placeAbove = spaceBelow < dropdownHeight && rect.top > spaceBelow;

    setCoords({
      top: placeAbove ? rect.top - 4 : rect.bottom + 4,
      left: rect.left,
      width: rect.width,
      placeAbove,
    });
  };

  useLayoutEffect(() => {
    if (isOpen) {
      updateCoords();
    }
  }, [isOpen, options.length]);

  // 监听窗口 resize 和 scroll
  useEffect(() => {
    if (!isOpen) return;

    const handleScrollOrResize = () => {
      updateCoords();
    };

    window.addEventListener("resize", handleScrollOrResize, true);
    window.addEventListener("scroll", handleScrollOrResize, true);
    return () => {
      window.removeEventListener("resize", handleScrollOrResize, true);
      window.removeEventListener("scroll", handleScrollOrResize, true);
    };
  }, [isOpen]);

  // 点击外部收起
  useEffect(() => {
    if (!isOpen) return;

    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        triggerRef.current &&
        !triggerRef.current.contains(target) &&
        dropdownRef.current &&
        !dropdownRef.current.contains(target)
      ) {
        setIsOpen(false);
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setIsOpen(false);
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen]);

  const handleSelect = (option: SelectOption<T>) => {
    if (option.disabled) return;
    triggerRef.current?.focus();
    onChange(option.value);
    setIsOpen(false);
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={isOpen}
        aria-controls={isOpen ? listId : undefined}
        aria-activedescendant={isOpen && activeIndex >= 0 ? `${listId}-${activeIndex}` : undefined}
        aria-label={ariaLabel}
        className={`custom-select-trigger ${isOpen ? "active" : ""} ${className}`}
        style={style}
        onClick={() => isOpen ? setIsOpen(false) : open()}
        onBlur={() => setIsOpen(false)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            if (!isOpen) open();
            else setActiveIndex(nextEnabledIndex(options, activeIndex, event.key === "ArrowDown" ? 1 : -1));
          } else if (isOpen && (event.key === "Home" || event.key === "End")) {
            event.preventDefault();
            setActiveIndex(event.key === "Home" ? nextEnabledIndex(options, -1, 1) : nextEnabledIndex(options, 0, -1));
          } else if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            if (!isOpen) open();
            else if (activeIndex >= 0 && options[activeIndex]) handleSelect(options[activeIndex]);
          } else if (isOpen && event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            setIsOpen(false);
          } else if (event.key === "Tab") setIsOpen(false);
        }}
      >
        <span className="custom-select-label">
          {selectedOption ? selectedOption.label : placeholder}
        </span>
        <ChevronDown size={13} className={`custom-select-chevron ${isOpen ? "open" : ""}`} />
      </button>

      {isOpen &&
        createPortal(
          <div
            ref={dropdownRef}
            id={listId}
            role="listbox"
            aria-label={ariaLabel}
            onMouseDown={(event) => event.preventDefault()}
            className={`custom-select-dropdown ${coords.placeAbove ? "place-above" : ""}`}
            style={{
              position: "fixed",
              left: `${coords.left}px`,
              top: coords.placeAbove ? "auto" : `${coords.top}px`,
              bottom: coords.placeAbove ? `${window.innerHeight - coords.top}px` : "auto",
              minWidth: `${coords.width}px`,
              zIndex: 99999,
            }}
          >
            <div className="custom-select-options">
              {options.map((opt, index) => {
                const isSelected = opt.value === value;
                return (
                  <div
                    key={String(opt.value)}
                    id={`${listId}-${index}`}
                    role="option"
                    aria-selected={isSelected}
                    aria-disabled={Boolean(opt.disabled)}
                    className={`custom-select-option ${isSelected ? "selected" : ""} ${
                      opt.disabled ? "disabled" : activeIndex === index ? "keyboard-active" : ""
                    }`}
                    onClick={() => handleSelect(opt)}
                    onMouseEnter={() => { if (!opt.disabled) setActiveIndex(index); }}
                  >
                    <span className="custom-select-option-label">{opt.label}</span>
                    {isSelected && <Check size={13} className="custom-select-check" />}
                  </div>
                );
              })}
            </div>
          </div>,
          document.body
        )}
    </>
  );
}
