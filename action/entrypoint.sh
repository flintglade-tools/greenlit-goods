#!/bin/sh
set -eu

if [ "$#" -ne 7 ]; then
    echo "::error title=Greenlit Goods configuration::The action received an invalid argument set." >&2
    exit 2
fi

feed=$1
format=$2
country=$3
destination=$4
assumed_monthly_sales=$5
strict=$6
json=$7

if [ -z "$feed" ]; then
    echo "::error title=Greenlit Goods configuration::The feed input is required." >&2
    exit 2
fi

case "$format" in
    ""|xml|csv) ;;
    *)
        echo "::error title=Greenlit Goods configuration::format must be xml or csv." >&2
        exit 2
        ;;
esac

case "$destination" in
    shopping-ads|free-listings) ;;
    *)
        echo "::error title=Greenlit Goods configuration::destination must be shopping-ads or free-listings." >&2
        exit 2
        ;;
esac

case "$strict" in
    true|false) ;;
    *)
        echo "::error title=Greenlit Goods configuration::strict must be true or false." >&2
        exit 2
        ;;
esac

case "$json" in
    true|false) ;;
    *)
        echo "::error title=Greenlit Goods configuration::json must be true or false." >&2
        exit 2
        ;;
esac

set -- audit "$feed" \
    --country "$country" \
    --destination "$destination" \
    --assumed-monthly-sales "$assumed_monthly_sales" \
    --no-color

if [ -n "$format" ]; then
    set -- "$@" --format "$format"
fi
if [ "$strict" = true ]; then
    set -- "$@" --strict
fi
if [ "$json" = true ]; then
    set -- "$@" --json
fi

exec greenlit "$@"
